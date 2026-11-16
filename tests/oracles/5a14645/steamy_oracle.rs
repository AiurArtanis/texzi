//! Independent tests added only to an isolated exported candidate. No product edits.
use texzi_document::*;
fn open(raw: &str) -> (Store, Snapshot) {
 let mut st=Store::new(HistoryBudget::default());
 let s=st.open_utf8(raw,Profile::default(),false).unwrap();
 (st,s)
}
fn tx(st:&mut Store,s:&Snapshot,edits:Vec<Edit>)->Receipt {
 st.submit(Transaction{doc:s.doc,base:s.revision,edits,boundary:BoundaryPolicy::Grapheme,cancelled:false}).unwrap()
}
fn apply_plain(mut value:String,delta:&[Delta])->String {
 // Coordinates all refer to one base string. Reverse order, not product code.
 for d in delta.iter().rev(){value.replace_range(d.range.clone(),&d.text);}
 value
}
#[test]
fn adjacent_deletions_inverse_and_anchor_hand_oracle() {
 let (mut st,s)=open("abcdef");
 let r=tx(&mut st,&s,vec![Edit::input(&s,1..3,""),Edit::input(&s,3..5,"")]);
 assert_eq!(r.snapshot.body.source().to_string(),"af");
 assert_eq!(apply_plain("abcdef".into(),&r.forward),"af");
 assert_eq!(r.inverse.iter().map(|d|(d.range.clone(),d.text.as_str())).collect::<Vec<_>>(),
 vec![(1..1,"bc"),(1..1,"de")]);
 assert_eq!(apply_plain("af".into(),&r.inverse),"abcdef");
 for (p,expected) in [(0,0),(1,1),(2,1),(3,1),(4,1),(5,1),(6,2)] {
  for affinity in [Affinity::Before,Affinity::After] {
   assert_eq!(r.map_anchor(SourceByte::at(&s,p),affinity).unwrap().offset,expected,"old byte {p}");
  }
 }
 let u=st.undo(s.doc).unwrap();
 assert_eq!(u.snapshot.body.source().to_string(),"abcdef");
 assert_eq!(u.snapshot.token,s.token);
 assert_eq!(u.snapshot.revision.0,2);
 assert_eq!(u.map_anchor(SourceByte::at(&r.snapshot,1),Affinity::Before).unwrap().offset,1);
 assert_eq!(u.map_anchor(SourceByte::at(&r.snapshot,1),Affinity::After).unwrap().offset,5);
 assert_eq!(apply_plain("af".into(),&u.forward),"abcdef");
 let redo=st.redo(s.doc).unwrap();
 assert_eq!(redo.snapshot.body.source().to_string(),"af");
 assert_eq!(redo.snapshot.token,r.snapshot.token);
 assert_eq!(redo.snapshot.revision.0,3);
}
#[test]
fn adjacent_replacements_inverse_at_shared_boundary() {
 let (mut st,s)=open("abcdef");
 let r=tx(&mut st,&s,vec![Edit::input(&s,1..3,"X"),Edit::input(&s,3..5,"YZ")]);
 assert_eq!(r.snapshot.body.source().to_string(),"aXYZf");
 assert_eq!(r.inverse.iter().map(|d|(d.range.clone(),d.text.as_str())).collect::<Vec<_>>(),
 vec![(1..2,"bc"),(2..4,"de")]);
 assert_eq!(apply_plain("aXYZf".into(),&r.inverse),"abcdef");
 for (p,before,after) in [(0,0,0),(1,1,2),(2,1,2),(3,2,4),(4,2,4),(5,4,4),(6,5,5)] {
  assert_eq!(r.map_anchor(SourceByte::at(&s,p),Affinity::Before).unwrap().offset,before);
  assert_eq!(r.map_anchor(SourceByte::at(&s,p),Affinity::After).unwrap().offset,after);
 }
}
fn normalize_bytes(raw:&[u8])->Vec<u8>{
 let mut out=Vec::new(); let mut i=0;
 while i<raw.len(){
  if raw[i]==b'\r'{out.push(b'\n');i+=1;if i<raw.len()&&raw[i]==b'\n'{i+=1;}}
  else{out.push(raw[i]);i+=1;}
 } out
}
fn check_origins(s:&Snapshot,expected:&[u8]){
 let mut end=0;
 for (span,origin) in s.body.origins(){
  assert_eq!(span.start,end,"gap/overlap");
  if let Some(o)=origin{
   assert!(o.source.is_char_boundary(o.raw.start)&&o.source.is_char_boundary(o.raw.end));
   let bytes=o.source.as_bytes();
   assert!(!(o.raw.start>0&&o.raw.start<bytes.len()&&bytes[o.raw.start-1]==b'\r'&&bytes[o.raw.start]==b'\n'));
   assert!(!(o.raw.end>0&&o.raw.end<bytes.len()&&bytes[o.raw.end-1]==b'\r'&&bytes[o.raw.end]==b'\n'));
   let normalized=normalize_bytes(&bytes[o.raw.clone()]);
   assert_eq!(normalized.len(),o.logical_len);
   assert_eq!(normalized,&expected[span.clone()]);
  }
  end=span.end;
 }
 assert_eq!(end,expected.len());
}
#[test]
fn mixed_eol_exact_origin_spans_without_product_decoder_oracle(){
 let raw="\u{feff}one\r\ntwo\nthree\rfour";
 let (mut st,s)=open(raw);
 check_origins(&s,b"one\ntwo\nthree\nfour");
 let r=tx(&mut st,&s,vec![Edit::input(&s,4..7,"TWO")]);
 assert_eq!(r.snapshot.body.source().to_string(),"one\nTWO\nthree\nfour");
 assert_eq!(r.snapshot.body.to_utf8(),b"\xef\xbb\xbfone\r\nTWO\nthree\rfour");
 assert_eq!(r.snapshot.body.eols(),vec![Eol::CrLf,Eol::Lf,Eol::Cr]);
 let spans=r.snapshot.body.origins();
 assert_eq!(spans.len(),3);
 assert_eq!(spans[0].0,0..4);assert_eq!(spans[0].1.as_ref().unwrap().raw,0..5);
 assert_eq!(spans[1].0,4..7);assert!(spans[1].1.is_none());
 assert_eq!(spans[2].0,7..18);assert_eq!(spans[2].1.as_ref().unwrap().raw,8..19);
 check_origins(&r.snapshot,b"one\nTWO\nthree\nfour");
 let n=tx(&mut st,&r.snapshot,vec![Edit::input(&r.snapshot,7..7,"\n")]);
 assert_eq!(n.snapshot.body.to_utf8(),b"\xef\xbb\xbfone\r\nTWO\r\n\nthree\rfour");
 check_origins(&n.snapshot,b"one\nTWO\n\nthree\nfour");
 let u1=st.undo(s.doc).unwrap();check_origins(&u1.snapshot,b"one\nTWO\nthree\nfour");
 let u2=st.undo(s.doc).unwrap();assert_eq!(u2.snapshot.body.to_utf8(),raw.as_bytes());
 assert_eq!(u2.snapshot.token,s.token);check_origins(&u2.snapshot,b"one\ntwo\nthree\nfour");
}
#[test]
fn long_original_span_unicode_crlf_seams_independent_bytes(){
 let raw=format!("{}猫\r\n😀\ne\u{301}\rZ","x".repeat(1023));
 let (mut st,s)=open(&raw);
 let old=normalize_bytes(raw.as_bytes());
 check_origins(&s,&old);
 let pos=String::from_utf8(old.clone()).unwrap().find("😀").unwrap();
 let r=tx(&mut st,&s,vec![Edit::input(&s,pos..pos+4,"Q\r\n")]);
 let mut expected=old.clone();expected.splice(pos..pos+4,b"Q\n".iter().copied());
 assert_eq!(r.snapshot.body.source().to_string().as_bytes(),expected);
 check_origins(&r.snapshot,&expected);
 assert_eq!(s.body.to_utf8(),raw.as_bytes());
 let u=st.undo(s.doc).unwrap();assert_eq!(u.snapshot.body.to_utf8(),raw.as_bytes());check_origins(&u.snapshot,&old);
}
#[test]
fn frozen_scalar_all_offsets_and_grapheme_table_independent_oracle(){
 let raw="A😀e\u{301}👩🏽‍💻🇨🇳B";
 let (_,s)=open(raw);
 let idx=GraphemeIndex::new(&s);
 let boundaries=[0,1,5,8,23,31,32];
 for b in 0..=raw.len()+1 {
  let p=SourceByte::at(&s,b);
  if b>raw.len() || !raw.is_char_boundary(b){
   assert_eq!(s.byte_to_utf16(p),Err(Error::InvalidBoundary));
   assert_eq!(idx.from_byte(p),Err(Error::InvalidBoundary));
  }else{
   assert_eq!(s.byte_to_utf16(p).unwrap().offset,raw[..b].encode_utf16().count());
   match boundaries.iter().position(|&n|n==b){
    Some(g)=>assert_eq!(idx.from_byte(p).unwrap().offset,g),
    None=>assert_eq!(idx.from_byte(p),Err(Error::InsideGrapheme))
   }
  }
 }
 for u in 0..=18 {
  let expected=raw.char_indices().map(|(b,_)|b).chain(std::iter::once(raw.len()))
   .find(|&b|raw[..b].encode_utf16().count()==u);
  match expected{
   Some(b)=>assert_eq!(s.utf16_to_byte(Utf16Offset::at(&s,u)).unwrap().offset,b),
   None=>assert_eq!(s.utf16_to_byte(Utf16Offset::at(&s,u)),Err(Error::InvalidBoundary))
  }
 }
}

