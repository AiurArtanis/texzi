//! The interpreter and all its native allocations live only in the isolated worker.
use std::{
    ffi::CString,
    ops::Range,
    ptr::{self, NonNull},
};

use pcre2_sys::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) static COMPILATIONS: AtomicU64 = AtomicU64::new(0);

use super::{
    RegexError, TemplateIssue,
    template::{Group, Parsed},
};

struct CompileContext(NonNull<pcre2_compile_context_8>);
impl Drop for CompileContext {
    fn drop(&mut self) {
        unsafe { pcre2_compile_context_free_8(self.0.as_ptr()) }
    }
}

struct Code(NonNull<pcre2_code_8>);
impl Drop for Code {
    fn drop(&mut self) {
        unsafe { pcre2_code_free_8(self.0.as_ptr()) }
    }
}

struct Data(NonNull<pcre2_match_data_8>);
impl Drop for Data {
    fn drop(&mut self) {
        unsafe { pcre2_match_data_free_8(self.0.as_ptr()) }
    }
}

struct Context(NonNull<pcre2_match_context_8>);
impl Drop for Context {
    fn drop(&mut self) {
        unsafe { pcre2_match_context_free_8(self.0.as_ptr()) }
    }
}

pub(super) struct Pattern {
    code: Code,
    data: Data,
    context: Context,
    slots: usize,
    single_atom: bool,
    ascii: Option<[bool; 128]>,
}

// These forms consume exactly one scalar, have no captures, and cannot inspect
// any context. This recognizer never supplies their matching semantics; PCRE2
// evaluates every ASCII input to the original compiled pattern below.
fn single_atom(pattern: &str) -> bool {
    match pattern.as_bytes() {
        [ch] => ch.is_ascii_alphanumeric() || b"._ -".contains(ch),
        [b'\\', ch] => b"dDsSwWhHvVntrf".contains(ch),
        [b'[', contents @ .., b']'] => {
            !contents.is_empty()
                && contents
                    .iter()
                    .all(|ch| ch.is_ascii_alphanumeric() || b" _^-".contains(ch))
        }
        _ => false,
    }
}

impl Pattern {
    pub fn compile(pattern: &str, case_sensitive: bool) -> Result<Self, RegexError> {
        if pattern.len() > super::MAX_EXPRESSION_BYTES {
            return Err(RegexError::Resource);
        }
        // Interpret the original pattern directly so compile errors point at its UTF-8 bytes.
        let mut options = PCRE2_UTF | PCRE2_UCP | PCRE2_MULTILINE | PCRE2_NEVER_BACKSLASH_C;
        if !case_sensitive {
            options |= PCRE2_CASELESS;
        }
        let (mut error, mut offset) = (0, 0);
        unsafe {
            let cc = CompileContext(
                NonNull::new(pcre2_compile_context_create_8(ptr::null_mut()))
                    .ok_or(RegexError::Resource)?,
            );
            pcre2_set_newline_8(cc.0.as_ptr(), PCRE2_NEWLINE_LF);
            pcre2_set_max_pattern_compiled_length_8(cc.0.as_ptr(), 4 * 1024 * 1024);
            let code = Code(
                NonNull::new(pcre2_compile_8(
                    pattern.as_ptr(),
                    pattern.len(),
                    options,
                    &mut error,
                    &mut offset,
                    cc.0.as_ptr(),
                ))
                .ok_or_else(|| match error as u32 {
                    PCRE2_ERROR_HEAP_FAILED
                    | PCRE2_ERROR_PATTERN_TOO_LARGE
                    | PCRE2_ERROR_PATTERN_STRING_TOO_LONG
                    | PCRE2_ERROR_TOO_MANY_CAPTURES => RegexError::Resource,
                    _ => RegexError::Syntax {
                        code: error,
                        byte: offset,
                    },
                })?,
            );
            let data = Data(
                NonNull::new(pcre2_match_data_create_from_pattern_8(
                    code.0.as_ptr(),
                    ptr::null_mut(),
                ))
                .ok_or(RegexError::Resource)?,
            );
            let context = Context(
                NonNull::new(pcre2_match_context_create_8(ptr::null_mut()))
                    .ok_or(RegexError::Resource)?,
            );
            pcre2_set_match_limit_8(context.0.as_ptr(), 1_000_000);
            pcre2_set_depth_limit_8(context.0.as_ptr(), 1_000);
            pcre2_set_heap_limit_8(context.0.as_ptr(), 32 * 1024); // KiB
            let slots = pcre2_get_ovector_count_8(data.0.as_ptr()) as usize;
            COMPILATIONS.fetch_add(1, Ordering::Relaxed);
            Ok(Self {
                code,
                data,
                context,
                slots,
                single_atom: single_atom(pattern),
                ascii: None,
            })
        }
    }

    pub fn prepare_ascii(&mut self, text: &str) -> Result<(), RegexError> {
        if !self.single_atom || text.len() < 256 || !text.is_ascii() {
            return Ok(());
        }
        let mut set = [false; 128];
        for code in 0..128u8 {
            let bytes = [code];
            let scalar = std::str::from_utf8(&bytes).unwrap();
            if let Some(hit) = self.next(scalar, 0)? {
                if hit.group(0) != Some(0..1) {
                    return Err(RegexError::Worker);
                }
                set[code as usize] = true;
            }
        }
        self.ascii = Some(set);
        Ok(())
    }

    pub fn ascii_table(&self) -> Option<&[bool; 128]> {
        self.ascii.as_ref()
    }

    /// Resolve names before looking for the first match. (?J) alternatives use the
    /// first participating group in the engine's name table, including lookahead captures.
    pub fn resolve(&self, template: &Parsed<'_>) -> Result<Vec<Vec<usize>>, RegexError> {
        template
            .groups
            .iter()
            .map(|&(group, byte)| {
                let unknown = RegexError::Template {
                    byte,
                    issue: TemplateIssue::UnknownGroup,
                };
                match group {
                    Group::Number(n) if (n as usize) < self.slots => Ok(vec![n as usize]),
                    Group::Number(_) => Err(unknown),
                    Group::Name(name) => {
                        let name = CString::new(name).map_err(|_| unknown.clone())?;
                        let (mut first, mut last) = (ptr::null(), ptr::null());
                        let step = unsafe {
                            pcre2_substring_nametable_scan_8(
                                self.code.0.as_ptr(),
                                name.as_ptr().cast(),
                                &mut first,
                                &mut last,
                            )
                        };
                        if step < 2 {
                            return Err(unknown);
                        }
                        let mut groups = Vec::new();
                        // PCRE2 returns equal-sized entries from a single contiguous table.
                        loop {
                            let index =
                                unsafe { u16::from_be_bytes([*first, *first.add(1)]) } as usize;
                            if index >= self.slots {
                                return Err(RegexError::Worker);
                            }
                            groups.push(index);
                            if first == last {
                                break;
                            }
                            first = unsafe { first.add(step as usize) };
                        }
                        Ok(groups)
                    }
                }
            })
            .collect()
    }

    pub fn next<'a>(
        &'a mut self,
        text: &str,
        start: usize,
    ) -> Result<Option<Match<'a>>, RegexError> {
        let data = self.data.0.as_ptr();
        // The worker owns its validated UTF-8 String. Every continuation is derived
        // from a checked scalar boundary, independently of any later file changes.
        debug_assert!(text.is_char_boundary(start));
        let bytes = if text.is_empty() {
            b"\0".as_ptr()
        } else {
            text.as_ptr()
        };
        let status = unsafe {
            pcre2_match_8(
                self.code.0.as_ptr(),
                bytes,
                text.len(),
                start,
                PCRE2_NO_UTF_CHECK,
                data,
                self.context.0.as_ptr(),
            )
        };
        match status {
            PCRE2_ERROR_NOMATCH => Ok(None),
            PCRE2_ERROR_MATCHLIMIT | PCRE2_ERROR_DEPTHLIMIT | PCRE2_ERROR_HEAPLIMIT => {
                Err(RegexError::EngineLimit { code: status })
            }
            PCRE2_ERROR_NOMEMORY => Err(RegexError::Resource),
            n if n > 0 => {
                // Only 0..n from the most recent match are meaningful.
                let offsets = unsafe {
                    std::slice::from_raw_parts(pcre2_get_ovector_pointer_8(data), 2 * n as usize)
                };
                Ok(Some(Match { offsets }))
            }
            code => Err(RegexError::Engine { code }),
        }
    }
}

pub(super) struct Match<'a> {
    offsets: &'a [usize],
}

impl<'a> Match<'a> {
    pub fn single(pair: &'a [usize; 2]) -> Self {
        Self { offsets: pair }
    }

    pub fn group(&self, slot: usize) -> Option<Range<usize>> {
        let pair = self.offsets.get(2 * slot..2 * slot + 2)?;
        (pair[0] != usize::MAX).then(|| pair[0]..pair[1])
    }
}

pub(super) fn error_text(code: i32) -> String {
    let mut bytes = [0; 256];
    let count = unsafe { pcre2_get_error_message_8(code, bytes.as_mut_ptr(), bytes.len()) };
    if count > 0 {
        String::from_utf8_lossy(&bytes[..count as usize]).into_owned()
    } else {
        format!("PCRE2 ({code})")
    }
}
