//! Windows file writes for the editor.
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::ERROR_UNABLE_TO_MOVE_REPLACEMENT_2,
    Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW},
};

/// When replacing an open document, expected holds its last on-disk bytes.
pub fn write(path: &Path, bytes: &[u8], expected: Option<&[u8]>) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "缺少文件名"))?;
    if name.encode_wide().any(|unit| unit == 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "文件名含空字符",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "缺少目标目录"))?
        .canonicalize()?;
    let target = parent.join(name);
    let (temp, mut file) = create_temp(&parent)?;
    let write = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    write?;
    if let Some(expected) = expected {
        if fs::metadata(&target)?.permissions().readonly() {
            return Err(io::Error::from_raw_os_error(5));
        }
        if fs::read(&target)? != expected {
            return Err(io::Error::other(
                "磁盘文件已被修改，请另存为以保留两份内容。",
            ));
        }
        replace(&target, &temp.0)
    } else {
        move_file(&temp.0, &target)
    }
}

fn create_temp(parent: &Path) -> io::Result<(TempPath, File)> {
    loop {
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
        let nonce = u128::from_ne_bytes(random);
        let path = parent.join(format!(".texzi-{nonce:032x}.tmp"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((TempPath(path), file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

struct TempPath(PathBuf);

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    let from = wide(from);
    let to = wide(to);
    // SAFETY: Both paths are NUL-terminated and remain alive for the call.
    let result = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn replace(target: &Path, temp: &Path) -> io::Result<()> {
    // ReplaceFileW can return 1177 after moving the original to the backup name.
    let mut backup_name = OsString::from(temp.as_os_str());
    backup_name.push(".bak");
    let backup = PathBuf::from(backup_name);
    if backup.try_exists()? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "临时备份名已被占用",
        ));
    }
    let target_wide = wide(target);
    let temp_wide = wide(temp);
    let backup_wide = wide(&backup);
    // SAFETY: All paths are NUL-terminated and the two reserved pointers are null.
    let result = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            temp_wide.as_ptr(),
            backup_wide.as_ptr(),
            0,
            null_mut(),
            null_mut(),
        )
    };
    if result != 0 {
        if let Err(error) = fs::remove_file(&backup) {
            eprintln!("无法删除保存备份 {}: {error}", backup.display());
        }
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2 as i32)
        && let Err(restore) = move_file(&backup, target)
    {
        return Err(io::Error::other(format!(
            "{error}\n恢复原路径失败：{restore}\n原文件保留在 {}",
            backup.display()
        )));
    }
    Err(error)
}
