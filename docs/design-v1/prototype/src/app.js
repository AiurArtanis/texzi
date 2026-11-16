'use strict';
/* texzi review prototype: browser implementation, not NSH production code.
   The build script injects the authoritative setting/command catalog below. */
const SETTING_CATALOG = window.TEXZI_SETTINGS || {settings:[]};
const COMMAND_CATALOG = window.TEXZI_COMMANDS || {commands:[]};
const TICK = String.fromCharCode(96);
const STORAGE_KEY = 'texzi.review.v1';
const ICONS = {
 file:'<path d="M5 3h9l5 5v13H5z"/><path d="M14 3v6h5M8 13h8M8 17h6"/>',
 folder:'<path d="M3 6h7l2 2h9v12H3z"/>',
 search:'<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
 settings:'<path d="m9 3-1 3-3 1-1 4 2 2v3l3 2 3-1 3 1 3-2v-3l2-2-1-4-3-1-1-3z"/><circle cx="12" cy="11" r="3"/>',
 split:'<rect x="3" y="4" width="18" height="16" rx="1.5"/><path d="M12 4v16"/>',
 plus:'<path d="M12 4v16M4 12h16"/>',
 close:'<path d="m6 6 12 12M18 6 6 18"/>',
 more:'<circle cx="5" cy="12" r=".8"/><circle cx="12" cy="12" r=".8"/><circle cx="19" cy="12" r=".8"/>',
 chevron:'<path d="m8 4 8 8-8 8"/>',
 down:'<path d="m5 9 7 7 7-7"/>',
 up:'<path d="m5 15 7-7 7 7"/>',
 code:'<path d="m8 7-5 5 5 5m8-10 5 5-5 5m-3-13-2 20"/>',
 palette:'<path d="M12 3a9 9 0 1 0 0 18h1a2 2 0 0 0 1-3l-1-1a2 2 0 0 1 1-3h3a4 4 0 0 0 4-4c-1-4-4-7-9-7z"/><circle cx="7" cy="10" r=".7"/><circle cx="10" cy="7" r=".7"/><circle cx="15" cy="7" r=".7"/>',
 window:'<rect x="3" y="4" width="18" height="16" rx="1"/><path d="M3 8h18M8 4v4"/>',
 keyboard:'<rect x="2" y="5" width="20" height="14" rx="1"/><path d="M6 9h1m3 0h1m3 0h1m3 0h1M6 12h1m3 0h1m3 0h1m3 0h1M7 16h10"/>',
 markdown:'<rect x="2" y="5" width="20" height="14" rx="1"/><path d="M5 15V9l3 3 3-3v6m4-6v6m-2-2 2 2 2-2"/>',
 restore:'<path d="M4 10a8 8 0 1 1 1 8M4 4v6h6M12 7v5l3 2"/>',
 external:'<path d="M14 3h7v7m0-7L10 14M10 4H4v16h16v-6"/>',
 sun:'<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1 1m12 12 1 1M5 19l1-1M18 6l1-1"/>',
 zen:'<path d="M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5"/>',
 save:'<path d="M4 3h14l3 3v15H3V3zM7 3v6h10V3M7 21v-8h10v8"/>',
 pin:'<path d="m8 3 9 3-2 6 3 4-7-2-4 4v-7zM9 15l-5 6"/>'
};
function icon(name){return '<svg viewBox="0 0 24 24" aria-hidden="true">'+(ICONS[name]||ICONS.file)+'</svg>';}
function esc(value){return String(value==null?'':value).replace(/[&<>"']/g,function(c){return {'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c];});}
function el(id){return document.getElementById(id);}
function textBytes(s){return new TextEncoder().encode(s).length;}
function stripEnding(s){return s.replace(/\r?\n$/,'');}
function languageFor(name){const x=name.toLowerCase();if(/\.(md|markdown)$/.test(x))return'markdown';if(/\.json$/.test(x))return'json';if(/\.(js|ts|jsx|tsx)$/.test(x))return'javascript';if(/\.py$/.test(x))return'python';if(/\.rs$/.test(x))return'rust';if(/\.(html|xml)$/.test(x))return'html';if(/\.css$/.test(x))return'css';return'plaintext';}
function languageLabel(lang){return{markdown:'Markdown',json:'JSON',javascript:'JavaScript',python:'Python',rust:'Rust',html:'HTML',css:'CSS',plaintext:'纯文本'}[lang]||lang;}
function initialDocuments(){
 const todo=['今日待办','','一、产品与文档','整理文本编辑器需求','确认 Markdown 所见即所得体验','复核窗口与标签操作','','二、会议记录','时间：09:30—10:00','主题：首版功能范围','','临时文本打开就写，离开后可以恢复。','Markdown 可以直接编辑，也可以切换源码。','代码片段支持语法高亮。','','三、稍后处理','整理本周项目笔记','检查未保存的临时文本',''].join('\n');
 const md=['# texzi · 产品笔记','','日常文字、Markdown 与代码片段。','','## 本周计划','','- [x] 核对功能边界','- [ ] 确认概念设计','- [ ] 完成编辑器验证','','## 编辑方式','','**直接编辑**排版后的正文，随时切换源码。','','| 模式 | 用途 |','| --- | --- |','| 所见即所得 | 直接写作与排版 |','| 源码 | 精确编辑 Markdown |','',TICK.repeat(3)+'js','const app = "texzi";',TICK.repeat(3),'','> 工具保持安静，让内容容易找到。',''].join('\n');
 const json=['{','  "theme": "system",','  "fontSize": 16,','  "wordWrap": true,','  "restoreSession": true','}'].join('\n');
 return[
  makeDocument('todo','今日待办.txt','日常文字/工作笔记/今日待办.txt',todo),
  makeDocument('project','项目笔记.md','日常文字/工作笔记/项目笔记.md',md),
  makeDocument('snippet','片段.json','日常文字/片段/片段.json',json),
  makeDocument('scratch','未命名-1','', '随手记\n\n在这里暂存灵感、复制的文字和待整理的片段。',true),
  makeDocument('meeting','周会记录.md','日常文字/会议/周会记录.md','# 周会记录\n\n## 讨论事项\n\n- 确认首版范围\n- 安排 Windows 输入法验证\n\n下一次会议：周三。\n')
 ];
}
function makeDocument(id,name,path,source,untitled){
 return{id,name,path,source,savedSource:untitled?'':source,language:languageFor(name),revision:0,encoding:'UTF-8',eol:source.includes('\r\n')?'CRLF':'LF',untitled:!!untitled,past:[],future:[],readOnly:false,banner:null,pinned:false,preview:false,lastEdit:0,lastKind:'',recovered:false};
}
function makeGroup(id,docIds,active){return{id,tabs:docIds.slice(),active:active||docIds[0],modes:{},scrolls:{},selections:{}};}
const defaultSettings={};
SETTING_CATALOG.settings.forEach(function(s){defaultSettings[s.id]=s.default;});
Object.assign(defaultSettings,{
 'appearance.theme':defaultSettings['appearance.theme']||'system',
 'editor.fontSize':defaultSettings['editor.fontSize']||16,
 'editor.lineHeight':defaultSettings['editor.lineHeight']||1.5,
 'editor.lineNumbers':defaultSettings['editor.lineNumbers']!==false,
 'editor.fontFamily':defaultSettings['editor.fontFamily']||'Cascadia Mono, Consolas',
 'editor.wordWrap.text':'on','editor.wordWrap.markdown':'on','editor.wordWrap.code':'off',
 'markdown.defaultView':'wysiwyg','markdown.fontSize':16,'markdown.lineHeight':1.7,'markdown.contentWidth':760,
 'files.autoSave':'off','session.restoreOnStartup':true,'session.restoreUnsaved':true,
 'workbench.showStatusBar':true
});
let state={
 docs:initialDocuments(),groups:[makeGroup('g1',['todo','project','snippet','scratch'],'todo')],
 activeGroup:'g1',scene:'main',sidebar:'files',sidebarVisible:true,settingsCategory:'常用',
 settings:Object.assign({},defaultSettings),settingsSearch:'',find:null,searchResult:{matches:[],current:0,error:null},
 workspaceQuery:'',zen:false,composing:false,multi:null,menu:null,clipboard:'',viewFocus:null
};
let restoredSession=false;
try{
 const stored=JSON.parse(localStorage.getItem(STORAGE_KEY)||'null');
 if(stored&&stored.version===1){
  state.settings=Object.assign({},defaultSettings,stored.settings||{});
  if(state.settings['session.restoreOnStartup']&&stored.docs&&stored.docs.length){
   state.docs=stored.docs.map(function(d){return Object.assign(makeDocument(d.id,d.name,d.path,d.source,d.untitled),d,{past:[],future:[],lastEdit:0,lastKind:''});});
   state.groups=stored.groups&&stored.groups.length?stored.groups:[makeGroup('g1',[state.docs[0].id])];
   state.activeGroup=state.groups[0].id;
   state.scene=stored.scene==='split'?'split':'main';
   restoredSession=true;
  }
 }
}catch(e){}
function currentGroup(){return state.groups.find(function(g){return g.id===state.activeGroup;})||state.groups[0];}
function docById(id){return state.docs.find(function(d){return d.id===id;});}
function currentDoc(){return docById(currentGroup().active);}
function isDirty(d){return d.source!==d.savedSource;}
function groupMode(g,d){if(!d||d.language!=='markdown')return'source';return g.modes[d.id]||state.settings['markdown.defaultView']||'wysiwyg';}
function setMode(g,d,mode){g.modes[d.id]=mode;}
let persistTimer;
function schedulePersist(){clearTimeout(persistTimer);persistTimer=setTimeout(persist,250);}
function persist(){
 try{
  const docs=state.docs.filter(function(d){return !d.demoLarge;}).map(function(d){
   const copy=Object.assign({},d);delete copy.past;delete copy.future;delete copy.lastEdit;delete copy.lastKind;
   if(!state.settings['session.restoreUnsaved']&&isDirty(d))copy.source=d.savedSource;
   return copy;
  });
  const ids=new Set(docs.map(function(d){return d.id;}));
  const groups=state.groups.map(function(g){const c=Object.assign({},g);c.tabs=g.tabs.filter(function(id){return ids.has(id);});c.active=c.tabs.includes(g.active)?g.active:c.tabs[0];return c;}).filter(function(g){return g.tabs.length;});
  localStorage.setItem(STORAGE_KEY,JSON.stringify({version:1,docs,groups,scene:state.scene,settings:state.settings}));
 }catch(e){toast('浏览器未能保存演示会话；可用“文件 → 下载当前文件”保留内容。');}
}
function applySettings(){
 let theme=state.settings['appearance.theme'];
 const dark=theme==='dark'||(theme==='system'&&matchMedia('(prefers-color-scheme: dark)').matches);
 document.body.classList.toggle('dark',dark);
 document.body.classList.toggle('high-contrast',theme==='highContrast');
 const r=document.documentElement.style;
 r.setProperty('--font-size',Number(state.settings['editor.fontSize'])+'px');
 r.setProperty('--line-height',Math.round(Number(state.settings['editor.fontSize'])*Number(state.settings['editor.lineHeight']))+'px');
 r.setProperty('--font-code',state.settings['editor.fontFamily']+',"Microsoft YaHei",monospace');
 document.body.style.fontSize=Number(state.settings['appearance.interfaceFontSize']||14)+'px';
}
function toast(message){el('toast').textContent=message;el('toast').classList.add('visible');clearTimeout(toast.timer);toast.timer=setTimeout(function(){el('toast').classList.remove('visible');},4200);}
function rememberView(){
 document.querySelectorAll('.editor-group').forEach(function(node){
  const g=state.groups.find(function(x){return x.id===node.dataset.group;});if(!g)return;
  const d=docById(g.active);if(!d)return;
  const ta=node.querySelector('.source-input');
  if(ta){g.scrolls[d.id]={top:ta.scrollTop,left:ta.scrollLeft};g.selections[d.id]=[ta.selectionStart,ta.selectionEnd];}
  else{const r=node.querySelector('.rich-scroll');if(r)g.scrolls[d.id]={top:r.scrollTop,left:0};}
 });
}
function editDocument(d,next,kind,forceStep){
 if(d.readOnly){toast('此文档当前为只读。');return false;}
 if(next===d.source)return false;
 const now=Date.now();
 if(forceStep||kind!==d.lastKind||now-d.lastEdit>750){d.past.push(d.source);if(d.past.length>120)d.past.shift();}
 d.future=[];d.source=next;d.revision++;d.lastEdit=now;d.lastKind=kind;d.preview=false;
 schedulePersist();updateChrome();
 if(state.settings['files.autoSave']==='afterDelay'&&!d.untitled&&!d.demoLarge){
  clearTimeout(d.autoSaveTimer);d.autoSaveTimer=setTimeout(function(){d.savedSource=d.source;updateChrome();toast('原型保存点已更新；浏览器不会直接覆盖本机原文件。');},Number(state.settings['files.autoSaveDelay']||1000));
 }
 return true;
}
function historyAction(redo){
 const d=currentDoc();if(!d)return;
 if(d.readOnly){toast('只读文档不能撤销或重做内容修改。');return;}
 const from=redo?d.future:d.past,to=redo?d.past:d.future;
 if(!from.length){toast(redo?'没有可重做的编辑':'没有可撤销的编辑');return;}
 rememberView();to.push(d.source);d.source=from.pop();d.revision++;d.lastKind='';d.lastEdit=0;
 state.multi=null;renderWorkspace();updateChrome();schedulePersist();if(state.find)runFind();
}
function sourceLines(src){
 const rows=[];let at=0;
 const parts=src.match(/[^\n]*(?:\n|$)/g)||[];
 parts.forEach(function(raw){if(raw==='')return;rows.push({raw,text:raw.replace(/\r?\n$/,''),start:at,end:at+raw.length});at+=raw.length;});
 return rows;
}
function parseMarkdown(src){
 const lines=sourceLines(src),blocks=[];let i=0;
 function push(type,from,to,extra){blocks.push(Object.assign({id:'b'+blocks.length,type,start:lines[from].start,end:lines[to-1].end,raw:src.slice(lines[from].start,lines[to-1].end)},extra||{}));}
 while(i<lines.length){
  if(!lines[i].text.trim()){i++;continue;}
  const start=i,t=lines[i].text;
  if(i===0&&t==='---'){i++;while(i<lines.length&&lines[i].text!=='---')i++;if(i<lines.length)i++;push('raw',start,i,{label:'Front matter · 原文保留'});continue;}
  const fence=t.match(/^(\x60{3,}|~{3,})(.*)$/);
  if(fence){i++;while(i<lines.length&&!lines[i].text.startsWith(fence[1]))i++;const close=i<lines.length;if(close)i++;push('code',start,i,{fence:fence[1],lang:fence[2].trim(),text:lines.slice(start+1,close?i-1:i).map(function(l){return l.text;}).join('\n')});continue;}
  const heading=t.match(/^(#{1,6})[ \t]+(.*)$/);
  if(heading){i++;push('heading',start,i,{level:heading[1].length,text:heading[2]});continue;}
  if(/^\s*[-*+]\s+\[[ xX]\]\s/.test(t)){
   i++;while(i<lines.length&&/^\s*[-*+]\s+\[[ xX]\]\s/.test(lines[i].text))i++;
   push('tasks',start,i,{items:lines.slice(start,i).map(function(l){const m=l.text.match(/^(\s*[-*+]\s+)\[([ xX])\](\s+)(.*)$/);return{checked:m[2]!==' ',text:m[4],prefix:m[1],space:m[3]};})});continue;
  }
  if(i+1<lines.length&&t.includes('|')&&/^\s*\|?\s*:?-{3,}/.test(lines[i+1].text)){
   i+=2;while(i<lines.length&&lines[i].text.trim()&&lines[i].text.includes('|'))i++;
   const cells=function(s){return s.trim().replace(/^\|/,'').replace(/\|$/,'').split('|').map(function(x){return x.trim();});};
   push('table',start,i,{rows:[cells(t)].concat(lines.slice(start+2,i).map(function(l){return cells(l.text);})),separator:lines[start+1].text});continue;
  }
  if(/^\s*([-*+]|\d+\.)\s+/.test(t)){
   i++;while(i<lines.length&&/^\s*([-*+]|\d+\.)\s+/.test(lines[i].text))i++;
   push('list',start,i,{ordered:/^\s*\d+\./.test(t),items:lines.slice(start,i).map(function(l){const m=l.text.match(/^(\s*(?:[-*+]|\d+\.)\s+)(.*)$/);return{prefix:m[1],text:m[2]};})});continue;
  }
  if(/^>\s?/.test(t)){i++;while(i<lines.length&&/^>\s?/.test(lines[i].text))i++;push('quote',start,i,{text:lines.slice(start,i).map(function(l){return l.text.replace(/^>\s?/,'');}).join('\n')});continue;}
  if(/^(-{3,}|\*{3,}|_{3,})\s*$/.test(t)){i++;push('hr',start,i);continue;}
  if(/^(<|:::|\$\$|!\[|\[\^[^\]]+\]:)/.test(t)){
   i++;while(i<lines.length&&lines[i].text.trim())i++;push('raw',start,i,{label:'特殊语法 · 原文保留'});continue;
  }
  i++;while(i<lines.length&&lines[i].text.trim()&&!/^(#{1,6}\s|\x60{3,}|~{3,}|>\s|[-*+]\s|\d+\.\s)/.test(lines[i].text))i++;
  push('paragraph',start,i,{text:lines.slice(start,i).map(function(l){return l.text;}).join('\n')});
 }
 return blocks;
}
function inlineMarkdown(s){
 const codes=[];
 let out=esc(s).replace(/\x60([^\x60]+)\x60/g,function(_,v){codes.push('<code>'+v+'</code>');return'\uE000'+(codes.length-1)+'\uE001';});
 out=out.replace(/\*\*([^*]+)\*\*/g,'<strong>$1</strong>').replace(/__([^_]+)__/g,'<strong>$1</strong>');
 out=out.replace(/(^|[^*])\*([^*]+)\*/g,'$1<em>$2</em>').replace(/~~([^~]+)~~/g,'<del>$1</del>');
 out=out.replace(/\[([^\]]+)\]\(([^)]+)\)/g,'<a href="#" data-link="$2">$1</a>');
 out=out.replace(/\uE000(\d+)\uE001/g,function(_,n){return codes[Number(n)];});
 return out.replace(/\n/g,'<br>');
}
function domInline(node){
 if(node.nodeType===Node.TEXT_NODE)return node.nodeValue;
 if(node.nodeType!==Node.ELEMENT_NODE)return'';
 const tag=node.tagName.toLowerCase();
 if(tag==='br')return'\n';
 let s=Array.from(node.childNodes).map(domInline).join('');
 if(tag==='strong'||tag==='b')return'**'+s+'**';
 if(tag==='em'||tag==='i')return'*'+s+'*';
 if(tag==='del'||tag==='s')return'~~'+s+'~~';
 if(tag==='code')return TICK+s+TICK;
 if(tag==='a')return'['+s+']('+(node.getAttribute('data-link')||'#')+')';
 if(tag==='div'||tag==='p')return s+'\n';
 return s;
}
function serializeBlock(node,b){
 const eol=b.raw.includes('\r\n')?'\r\n':'\n';
 const ending=/\r?\n$/.test(b.raw)?eol:'';
 let content='';
 if(b.type==='heading')content='#'.repeat(b.level)+' '+domInline(node).replace(/\n+$/,'');
 else if(b.type==='paragraph')content=domInline(node).replace(/\n+$/,'');
 else if(b.type==='quote')content=domInline(node).replace(/\n+$/,'').split('\n').map(function(x){return'> '+x;}).join(eol);
 else if(b.type==='code')content=b.fence+b.lang+eol+(node.querySelector('code').textContent||'').replace(/\n+$/,'')+eol+b.fence;
 else if(b.type==='tasks')content=Array.from(node.querySelectorAll('li')).map(function(li,j){return(b.items[j]?b.items[j].prefix:'- ')+'['+(li.querySelector('input').checked?'x':' ')+'] '+domInline(li.querySelector('.task-text')).replace(/\n+$/,'');}).join(eol);
 else if(b.type==='list')content=Array.from(node.children).map(function(li,j){return(b.items[j]?b.items[j].prefix:(b.ordered?(j+1)+'. ':'- '))+domInline(li).replace(/\n+$/,'');}).join(eol);
 else if(b.type==='table'){
  const rows=Array.from(node.querySelectorAll('tr')).map(function(tr){return'| '+Array.from(tr.children).map(function(td){return domInline(td).replace(/\n/g,' ').trim().replace(/\|/g,'\\|');}).join(' | ')+' |';});
  rows.splice(1,0,b.separator);content=rows.join(eol);
 }else if(b.type==='raw')content=node.querySelector('pre').textContent.replace(/\n+$/,'');
 else return b.raw;
 return content.replace(/\r?\n/g,eol)+ending;
}
function renderMarkdown(d){
 const blocks=parseMarkdown(d.source);
 return blocks.map(function(b){
  const data=' data-block="'+b.id+'" data-start="'+b.start+'" data-end="'+b.end+'"';
  const editable=d.readOnly?'false':'true';
  if(b.type==='heading')return'<h'+b.level+data+' contenteditable="'+editable+'" spellcheck="false">'+inlineMarkdown(b.text)+'</h'+b.level+'>';
  if(b.type==='paragraph')return'<p'+data+' contenteditable="'+editable+'" spellcheck="false">'+inlineMarkdown(b.text)+'</p>';
  if(b.type==='quote')return'<blockquote'+data+' contenteditable="'+editable+'" spellcheck="false">'+inlineMarkdown(b.text)+'</blockquote>';
  if(b.type==='code')return'<pre'+data+'><code contenteditable="'+editable+'" spellcheck="false">'+esc(b.text)+'</code></pre>';
  if(b.type==='tasks')return'<ul class="task-list"'+data+'>'+b.items.map(function(it){return'<li><input type="checkbox" class="task-box" '+(it.checked?'checked ':'')+(d.readOnly?'disabled ':'')+'aria-label="切换任务完成状态"><span class="task-text" contenteditable="'+editable+'" spellcheck="false">'+inlineMarkdown(it.text)+'</span></li>';}).join('')+'</ul>';
  if(b.type==='list')return'<'+(b.ordered?'ol':'ul')+data+'>'+b.items.map(function(it){return'<li contenteditable="'+editable+'" spellcheck="false">'+inlineMarkdown(it.text)+'</li>';}).join('')+'</'+(b.ordered?'ol':'ul')+'>';
  if(b.type==='table')return'<table'+data+'><tbody>'+b.rows.map(function(row,i){return'<tr>'+row.map(function(cell){return'<'+(i?'td':'th')+' contenteditable="'+editable+'" spellcheck="false">'+inlineMarkdown(cell)+'</'+(i?'td':'th')+'>';}).join('')+'</tr>';}).join('')+'</tbody></table>';
  if(b.type==='hr')return'<hr'+data+'>';
  return'<div class="raw-block"'+data+'><div class="raw-label">'+esc(b.label)+'</div><pre contenteditable="'+editable+'" spellcheck="false">'+esc(stripEnding(b.raw))+'</pre></div>';
 }).join('')||'<p data-block="empty" contenteditable="'+(!d.readOnly)+'" data-start="0" data-end="0" spellcheck="false"><br></p>';
}
function commitRichBlock(node,d){
 if(state.composing||d.readOnly)return;
 const start=Number(node.dataset.start),end=Number(node.dataset.end);
 const blocks=parseMarkdown(d.source),b=blocks.find(function(x){return x.start===start;});
 let replacement=b?serializeBlock(node,b):domInline(node).replace(/\n+$/,'');
 if(b&&replacement===b.raw)return;
 const next=d.source.slice(0,start)+replacement+d.source.slice(end),delta=replacement.length-(end-start);
 if(!editDocument(d,next,'rich:'+start,false))return;
 node.dataset.end=String(start+replacement.length);
 const root=node.closest('.rich-document');
 Array.from(root.querySelectorAll('[data-block]')).forEach(function(other){
  if(other===node)return;
  if(Number(other.dataset.start)>=end){other.dataset.start=String(Number(other.dataset.start)+delta);other.dataset.end=String(Number(other.dataset.end)+delta);}
 });
 updateOtherViews(d.id,node.closest('.editor-group').dataset.group);
 if(state.find)runFind();
}
function wrapFor(d){return state.settings[d.language==='plaintext'?'editor.wordWrap.text':d.language==='markdown'?'editor.wordWrap.markdown':'editor.wordWrap.code']==='on';}
function highlightSource(s,lang){
 if(lang==='plaintext'||lang==='markdown')return esc(s);
 const re=/("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')|\b(true|false|null|const|let|var|function|return|if|else|import|from|class|def|fn|pub|use|async|await)\b|\b(\d+(?:\.\d+)?)\b/g;
 let out='',at=0,m;
 while((m=re.exec(s))){out+=esc(s.slice(at,m.index));const key=m[1]&&/^\s*:/.test(s.slice(re.lastIndex));out+='<span class="'+(m[1]?(key?'kw':'str'):m[2]?'kw':'num')+'">'+esc(m[0])+'</span>';at=re.lastIndex;}
 return out+esc(s.slice(at));
}
function markedSource(d,g){
 const matches=state.find&&state.find.group===g.id&&state.find.doc===d.id?state.searchResult.matches:[];
 const multi=state.multi&&state.multi.group===g.id?state.multi.ranges:[];
 const spans=multi.length>1?multi.map(function(r){return{start:r[0],end:r[1],multi:true};}):matches;
 if(!spans.length)return highlightSource(d.source,d.language)+'\n';
 let out='',at=0;
 spans.slice(0,20000).forEach(function(m,i){
  if(m.start<at)return;out+=esc(d.source.slice(at,m.start));
  if(m.multi)out+='<span class="multi-selection">'+esc(d.source.slice(m.start,m.end))+'</span>';
  else out+='<mark class="'+(i===state.searchResult.current?'current':'')+'">'+esc(d.source.slice(m.start,m.end))+'</mark>';
  at=m.end;
 });
 return out+esc(d.source.slice(at))+'\n';
}
function renderSource(d,g){
 const wrap=wrapFor(d)?' wrap':'';
 const numbers=state.settings['editor.lineNumbers']?'<div class="line-numbers" aria-hidden="true">'+d.source.split('\n').map(function(_,i){return i+1;}).join('\n')+'</div>':'';
 return'<div class="source-wrap">'+numbers+'<div class="source-scroll"><pre class="source-highlight'+wrap+'" aria-hidden="true">'+markedSource(d,g)+'</pre><textarea class="source-input'+wrap+'" data-doc="'+esc(d.id)+'" aria-label="'+esc(d.name)+' 源码编辑器" spellcheck="false" autocapitalize="off" autocomplete="off" '+(d.readOnly?'readonly ':'')+'wrap="'+(wrap?'soft':'off')+'">'+esc(d.source)+'</textarea></div></div>';
}
function renderTabbar(g){
 let tabs=g.tabs.map(function(id){const d=docById(id);if(!d)return'';return'<div class="tab '+(g.active===id?'active ':'')+(d.preview?'preview ':'')+(d.pinned?'pinned ':'')+'" data-doc="'+esc(id)+'" draggable="true" role="tab" aria-selected="'+(g.active===id)+'" tabindex="0"><span class="tab-name">'+esc(d.name)+'</span>'+(isDirty(d)?'<span class="dirty" title="未保存"></span>':'')+'<button class="tab-close" data-close="'+esc(id)+'" title="关闭标签" aria-label="关闭 '+esc(d.name)+'">×</button></div>';}).join('');
 return'<div class="tabbar"><div class="tab-list" role="tablist">'+tabs+'</div><div class="tab-actions"><button class="icon-button group-add" title="新建文本" aria-label="新建文本">'+icon('plus')+'</button><button class="icon-button group-split" title="拆分编辑区域" aria-label="拆分编辑区域">'+icon('split')+'</button><button class="icon-button group-more" title="编辑组菜单" aria-label="编辑组菜单">'+icon('more')+'</button></div></div>';
}
function bannerHtml(d){
 if(!d.banner)return'';
 if(d.banner==='external')return'<div class="inline-banner"><span>文件在外部发生变化，当前编辑也有未保存修改。</span><button data-banner-action="compare">比较</button><button data-banner-action="reload">重新加载</button><button data-banner-action="keep">保留编辑</button></div>';
 if(d.banner==='recovery')return'<div class="inline-banner info"><span>已找到未保存内容的恢复副本。请核对后继续编辑。</span><button data-banner-action="accept-recovery">保留恢复内容</button><button data-banner-action="discard-recovery">放弃恢复</button></div>';
 if(d.banner==='large')return'<div class="inline-banner"><span>大文件模式 · 100 MiB 场景演示：文本保持可编辑，正文为小样本。</span><button data-banner-action="normal">退出演示</button><button data-banner-action="large-info">查看策略</button></div>';
 if(d.banner==='readonly')return'<div class="inline-banner info"><span>此文件为只读，仍可查找、选择和复制。</span><button data-banner-action="save-as">另存为</button><button data-banner-action="unlock">退出只读演示</button></div>';
 if(d.banner==='save-error')return'<div class="inline-banner error"><span>保存失败：目标目录不可写。修改仍保留在编辑器中。</span><button data-banner-action="save-as">另存为</button><button data-banner-action="retry">重试</button></div>';
 return'';
}
function findHtml(g,d){
 if(!state.find||state.find.group!==g.id||state.find.doc!==d.id)return'';
 const f=state.find,r=state.searchResult,scope=groupMode(g,d)==='wysiwyg'?'正文':'源码';
 return'<div class="find-widget"><div class="find-row"><button class="icon-button find-expand" title="'+(f.replace?'收起替换':'展开替换')+'" aria-label="展开替换">'+icon(f.replace?'down':'chevron')+'</button><span class="find-scope">'+scope+'</span><div class="find-field"><input class="find-query" aria-label="查找内容" value="'+esc(f.query)+'" placeholder="查找"><div class="find-options"><button data-find-option="case" class="'+(f.case?'active':'')+'" title="区分大小写">Aa</button><button data-find-option="word" class="'+(f.word?'active':'')+'" title="全字匹配">Ab|</button><button data-find-option="regex" class="'+(f.regex?'active':'')+'" title="正则表达式">.*</button></div></div><span class="find-result">'+(r.matches.length?(r.current+1)+' / '+r.matches.length:'无结果')+'</span><button class="icon-button find-prev" title="上一个" aria-label="上一个匹配">'+icon('up')+'</button><button class="icon-button find-next" title="下一个" aria-label="下一个匹配">'+icon('down')+'</button><button class="icon-button find-close" title="关闭" aria-label="关闭查找">'+icon('close')+'</button></div>'+(f.replace?'<div class="find-row"><div style="width:30px"></div><div class="find-field"><input class="replace-query" aria-label="替换为" value="'+esc(f.replacement||'')+'" placeholder="替换为"></div><div class="find-actions"><button class="replace-one">替换</button><button class="replace-all">全部替换</button></div></div>':'')+'<div class="find-error" '+(!r.error?'hidden':'')+'>'+esc(r.error||'')+'</div></div>';
}
function renderGroup(g){
 const d=docById(g.active);
 if(!d)return'<section class="editor-group" data-group="'+g.id+'">'+renderTabbar(g)+'<div class="empty-editor"><div><h1>texzi</h1><button data-empty="new">新建文本 <kbd>Ctrl N</kbd></button><button data-empty="open">打开文件 <kbd>Ctrl O</kbd></button><button data-empty="folder">打开文件夹 <kbd>Ctrl K Ctrl O</kbd></button></div></div></section>';
 const mode=groupMode(g,d),crumb=d.path||'未保存的临时文本';
 return'<section class="editor-group '+(state.activeGroup===g.id?'focused':'')+'" data-group="'+g.id+'">'+renderTabbar(g)+(mode==='source'?'<div class="breadcrumb">'+esc(crumb).split('/').map(function(x){return'<span>'+x+'</span>';}).join('<span>›</span>')+'</div>':'')+bannerHtml(d)+'<div class="editor-content">'+(d.language==='markdown'?'<button class="md-toggle '+(mode==='source'?'active':'')+'" aria-pressed="'+(mode==='source')+'" title="'+(mode==='source'?'返回所见即所得编辑':'切换到源码编辑')+'" aria-label="'+(mode==='source'?'返回所见即所得编辑':'切换到源码编辑')+'">&gt;_</button>':'')+(mode==='wysiwyg'?'<div class="rich-scroll"><article class="rich-document" aria-label="'+esc(d.name)+' 所见即所得编辑器">'+renderMarkdown(d)+'</article></div>':renderSource(d,g))+findHtml(g,d)+'</div></section>';
}
function renderWorkspace(){
 const w=el('workspace');
 if(state.scene==='settings'){renderSettings();return;}
 if(state.groups.length===1){w.innerHTML=renderGroup(state.groups[0]);}
 else{
  w.innerHTML='<div class="layout-split"><div class="split-left">'+renderGroup(state.groups[0])+'</div><div class="split-handle" role="separator" aria-label="调整左右编辑区域"></div><div class="split-right">'+state.groups.slice(1).map(function(g,i){return(i?'<div class="split-handle horizontal" role="separator" aria-label="调整上下编辑区域"></div>':'')+renderGroup(g);}).join('')+'</div></div>';
 }
 bindWorkspace();
 state.groups.forEach(function(g){
  const node=document.querySelector('[data-group="'+g.id+'"]');if(!node)return;
  const d=docById(g.active);if(!d)return;
  const pos=g.scrolls[d.id],sel=g.selections[d.id],ta=node.querySelector('.source-input');
  if(ta){if(sel){ta.selectionStart=Math.min(sel[0],ta.value.length);ta.selectionEnd=Math.min(sel[1],ta.value.length);}if(pos){ta.scrollTop=pos.top;ta.scrollLeft=pos.left;}syncSourceScroll(ta);}
  const rich=node.querySelector('.rich-scroll');if(rich&&pos)rich.scrollTop=pos.top;
  if(rich){rich.querySelector('.rich-document').style.fontSize=(state.settings['markdown.fontSize']||16)+'px';rich.querySelector('.rich-document').style.lineHeight=String(state.settings['markdown.lineHeight']||1.7);rich.querySelector('.rich-document').style.maxWidth=(state.settings['markdown.contentWidth']||760)+'px';}
 });
 updateStatus();
}
function syncSourceScroll(ta){
 const group=ta.closest('.editor-group'),pre=group.querySelector('.source-highlight'),numbers=group.querySelector('.line-numbers');
 if(pre){pre.scrollTop=ta.scrollTop;pre.scrollLeft=ta.scrollLeft;}
 if(numbers)numbers.scrollTop=ta.scrollTop;
}
function updateSourceView(ta,d,g){
 const pre=ta.parentElement.querySelector('.source-highlight');if(pre)pre.innerHTML=markedSource(d,g);
 const n=ta.closest('.editor-group').querySelector('.line-numbers');if(n)n.textContent=d.source.split('\n').map(function(_,i){return i+1;}).join('\n');
 syncSourceScroll(ta);
}
function updateOtherViews(docId,except){
 state.groups.forEach(function(g){
  if(g.active!==docId||g.id===except)return;
  const node=document.querySelector('[data-group="'+g.id+'"]');if(!node)return;
  const d=docById(docId),ta=node.querySelector('.source-input');
  if(ta){const a=ta.selectionStart,b=ta.selectionEnd;ta.value=d.source;ta.setSelectionRange(Math.min(a,ta.value.length),Math.min(b,ta.value.length));updateSourceView(ta,d,g);}
  else{const rich=node.querySelector('.rich-document');if(rich)rich.innerHTML=renderMarkdown(d);}
 });
}
function bindWorkspace(){
 document.querySelectorAll('.editor-group').forEach(function(node){
  const g=state.groups.find(function(x){return x.id===node.dataset.group;}),d=docById(g.active);
  node.addEventListener('pointerdown',function(){state.activeGroup=g.id;document.querySelectorAll('.editor-group').forEach(function(x){x.classList.toggle('focused',x===node);});updateStatus();});
  node.querySelectorAll('.tab').forEach(function(tab){
   tab.addEventListener('click',function(e){if(e.target.closest('.tab-close'))return;activateDoc(g.id,tab.dataset.doc);});
   tab.addEventListener('keydown',function(e){if(e.key==='Enter')activateDoc(g.id,tab.dataset.doc);});
   tab.addEventListener('dblclick',function(){const t=docById(tab.dataset.doc);t.preview=false;updateChrome();});
   tab.addEventListener('contextmenu',function(e){e.preventDefault();tabMenu(e,tab.dataset.doc,g.id);});
   tab.addEventListener('dragstart',function(e){e.dataTransfer.setData('application/x-texzi-tab',JSON.stringify({doc:tab.dataset.doc,group:g.id}));e.dataTransfer.effectAllowed='move';});
  });
  node.querySelectorAll('[data-close]').forEach(function(b){b.onclick=function(e){e.stopPropagation();closeTab(g.id,b.dataset.close);};});
  node.querySelector('.group-add').onclick=function(){state.activeGroup=g.id;newDocument();};
  node.querySelector('.group-split').onclick=function(){state.activeGroup=g.id;splitCurrent();};
  node.querySelector('.group-more').onclick=function(e){state.activeGroup=g.id;showMenu(e.currentTarget,[['拆分当前文档','Ctrl+\\',splitCurrent],['移动到新窗口','',newWindow],['固定当前标签','',togglePin],['关闭编辑区域','',function(){closeGroup(g.id);}]]);};
  node.addEventListener('dragover',function(e){if(Array.from(e.dataTransfer.types).includes('application/x-texzi-tab')){e.preventDefault();e.dataTransfer.dropEffect='move';}});
  node.addEventListener('drop',function(e){const raw=e.dataTransfer.getData('application/x-texzi-tab');if(!raw)return;e.preventDefault();const data=JSON.parse(raw);moveTab(data.group,g.id,data.doc);});
  node.querySelectorAll('[data-empty]').forEach(function(b){b.onclick=function(){perform(b.dataset.empty);};});
  if(!d)return;
  const toggle=node.querySelector('.md-toggle');if(toggle)toggle.onclick=function(){toggleMarkdown(g.id);};
  node.querySelectorAll('[data-banner-action]').forEach(function(b){b.onclick=function(){state.activeGroup=g.id;bannerAction(b.dataset.bannerAction,d);};});
  const ta=node.querySelector('.source-input');
  if(ta){
   ta.addEventListener('compositionstart',function(){state.composing=true;});
   ta.addEventListener('compositionend',function(){state.composing=false;editDocument(d,ta.value,'composition',true);updateSourceView(ta,d,g);updateOtherViews(d.id,g.id);if(state.find)runFind();});
   ta.addEventListener('beforeinput',function(e){if(state.multi&&state.multi.group===g.id&&state.multi.ranges.length>1&&!e.isComposing&&e.inputType==='insertText'){e.preventDefault();applyMulti(e.data||'');}});
   ta.addEventListener('input',function(){if(state.composing){updateSourceView(ta,Object.assign({},d,{source:ta.value}),g);return;}editDocument(d,ta.value,'source',false);updateSourceView(ta,d,g);updateOtherViews(d.id,g.id);if(state.find)runFind();updateStatus();});
   ta.addEventListener('scroll',function(){syncSourceScroll(ta);g.scrolls[d.id]={top:ta.scrollTop,left:ta.scrollLeft};});
   ta.addEventListener('select',function(){g.selections[d.id]=[ta.selectionStart,ta.selectionEnd];updateStatus();});
   ta.addEventListener('keyup',updateStatus);
   ta.addEventListener('click',function(e){state.activeGroup=g.id;if(e.altKey){addCursorAt(g,ta.selectionStart);}else if(state.multi){state.multi=null;updateSourceView(ta,d,g);}updateStatus();});
   ta.addEventListener('keydown',function(e){
    if(e.isComposing)return;
    if(e.key==='Tab'&&!e.ctrlKey&&!e.metaKey){e.preventDefault();replaceSelection(' '.repeat(Number(state.settings['editor.tabSize']||4)),true);}
    if(e.key==='Enter'&&!e.ctrlKey&&!e.altKey&&state.multi&&state.multi.ranges.length>1){e.preventDefault();applyMulti('\n');}
    if(e.key==='Backspace'&&state.multi&&state.multi.ranges.length>1){e.preventDefault();applyMulti('',true);}
   });
   ta.addEventListener('paste',function(e){if(state.multi&&state.multi.ranges.length>1){e.preventDefault();applyMulti(e.clipboardData.getData('text/plain'));}});
  }
  const rich=node.querySelector('.rich-document');
  if(rich){
   rich.addEventListener('compositionstart',function(){state.composing=true;});
   rich.addEventListener('compositionend',function(e){state.composing=false;const block=e.target.closest('[data-block]');if(block)commitRichBlock(block,d);});
   rich.addEventListener('input',function(e){const block=e.target.closest('[data-block]');if(block)commitRichBlock(block,d);});
   rich.addEventListener('change',function(e){if(e.target.classList.contains('task-box'))commitRichBlock(e.target.closest('[data-block]'),d);});
   rich.addEventListener('paste',function(e){
    if(d.readOnly)return;e.preventDefault();const value=e.clipboardData.getData('text/plain');document.execCommand('insertText',false,value);
    const block=e.target.closest('[data-block]');if(block)commitRichBlock(block,d);
   });
   rich.addEventListener('click',function(e){const a=e.target.closest('a');if(a){e.preventDefault();toast('链接地址：'+a.dataset.link+'（原型不打开外部网址）');}});
  }
  bindFind(node,g,d);
 });
 bindSplitHandles();
}

function renderSidebar(){
 const side=el('sidebar');
 side.style.display=state.sidebarVisible?'flex':'none';el('sidebar-resizer').style.display=state.sidebarVisible?'block':'none';
 el('activity-files').classList.toggle('active',state.scene!=='settings'&&state.sidebar==='files');
 el('activity-search').classList.toggle('active',state.scene!=='settings'&&state.sidebar==='search');
 el('activity-settings').classList.toggle('active',state.scene==='settings');
 if(state.scene==='settings'){
  const nav=[['常用','settings'],['编辑器','code'],['外观','palette'],['文件与恢复','folder'],['Markdown','markdown'],['窗口与标签','window'],['快捷键','keyboard']];
  side.innerHTML='<div class="sidebar-heading" style="font-size:17px;height:52px">设置</div><nav class="settings-nav">'+nav.map(function(item){return'<button data-category="'+item[0]+'" class="'+(state.settingsCategory===item[0]?'selected':'')+'">'+icon(item[1])+'<span>'+item[0]+'</span></button>';}).join('')+'</nav>';
  side.querySelectorAll('[data-category]').forEach(function(b){b.onclick=function(){state.settingsCategory=b.dataset.category;renderSidebar();renderSettings();};});
  return;
 }
 if(state.sidebar==='search'){
  side.innerHTML='<div class="sidebar-heading">文件夹搜索</div><div class="folder-search"><input id="workspace-query" placeholder="在已打开的演示文件中查找" aria-label="文件夹搜索内容" value="'+esc(state.workspaceQuery)+'"><div style="font-size:11px;color:var(--muted)">原型范围：已载入文件 · 普通文本匹配</div></div><div class="folder-results" id="folder-results"></div>';
  el('workspace-query').oninput=function(e){state.workspaceQuery=e.target.value;renderFolderResults();};renderFolderResults();return;
 }
 side.innerHTML='<div class="sidebar-heading"><span>资源管理器</span><div class="actions"><button class="icon-button" id="tree-new" title="新建文件" aria-label="新建文件">'+icon('plus')+'</button><button class="icon-button" id="tree-folder" title="打开文件夹" aria-label="打开文件夹">'+icon('folder')+'</button></div></div><div class="tree"><div class="tree-row level0"><span class="tree-arrow">⌄</span>'+icon('folder')+'<span class="file-name">'+esc(state.folderName||'日常文字')+'</span></div><div class="tree-row level1"><span class="tree-arrow">⌄</span>'+icon('folder')+'<span>工作笔记</span></div>'+state.docs.filter(function(d){return!d.untitled&&!/片段|会议/.test(d.path);}).map(function(d){return treeFile(d,2);}).join('')+'<div class="tree-row level1"><span class="tree-arrow">⌄</span>'+icon('folder')+'<span>会议</span></div>'+state.docs.filter(function(d){return/会议/.test(d.path);}).map(function(d){return treeFile(d,2);}).join('')+'<div class="tree-row level1"><span class="tree-arrow">⌄</span>'+icon('folder')+'<span>片段</span></div>'+state.docs.filter(function(d){return/片段/.test(d.path);}).map(function(d){return treeFile(d,2);}).join('')+'</div><div class="sidebar-section" style="margin-top:18px">临时文本</div><div class="tree">'+state.docs.filter(function(d){return d.untitled;}).map(function(d){return treeFile(d,1);}).join('')+'</div>';
 side.querySelectorAll('[data-tree-doc]').forEach(function(b){
  b.onclick=function(){openDocument(b.dataset.treeDoc,true);};
  b.ondblclick=function(){const d=docById(b.dataset.treeDoc);d.preview=false;updateChrome();};
 });
 el('tree-new').onclick=newDocument;el('tree-folder').onclick=function(){el('folder-input').click();};
}
function treeFile(d,level){return'<div class="tree-row level'+level+' '+(currentGroup().active===d.id?'active':'')+'" data-tree-doc="'+esc(d.id)+'" tabindex="0"><span class="tree-arrow"></span>'+icon(d.language==='markdown'?'markdown':d.language==='json'?'code':'file')+'<span class="file-name">'+esc(d.name)+'</span>'+(isDirty(d)?'<span class="dirty">•</span>':'')+'</div>';}
function renderFolderResults(){
 const box=el('folder-results');if(!box)return;
 const q=state.workspaceQuery.toLowerCase();if(!q){box.innerHTML='';return;}
 let total=0,out='';
 state.docs.forEach(function(d){
  d.source.split('\n').forEach(function(line,i){
   if(line.toLowerCase().includes(q)&&total<100){total++;out+='<button class="folder-result" data-doc="'+d.id+'" data-line="'+i+'"><span class="file">'+esc(d.name)+' · '+(i+1)+'</span><span class="snippet">'+esc(line)+'</span></button>';}
  });
 });
 box.innerHTML='<div style="padding:7px 10px;font-size:11px;color:var(--muted)">'+total+' 个结果'+(total===100?'（原型最多显示100项）':'')+'</div>'+out;
 box.querySelectorAll('[data-doc]').forEach(function(b){b.onclick=function(){openDocument(b.dataset.doc,false);const g=currentGroup(),d=currentDoc();setMode(g,d,'source');renderWorkspace();const ta=activeTextarea();if(ta){const n=Number(b.dataset.line),offset=d.source.split('\n').slice(0,n).join('\n').length+(n?1:0);ta.focus();ta.setSelectionRange(offset,offset+q.length);ta.scrollTop=n*24-70;syncSourceScroll(ta);}};});
}
function updateChrome(){
 if(state.scene!=='settings'){
  document.querySelectorAll('.tab[data-doc]').forEach(function(t){const d=docById(t.dataset.doc);if(!d)return;let dot=t.querySelector('.dirty');if(isDirty(d)&&!dot){dot=document.createElement('span');dot.className='dirty';dot.title='未保存';t.insertBefore(dot,t.querySelector('.tab-close'));}if(!isDirty(d)&&dot)dot.remove();t.classList.toggle('pinned',d.pinned);t.classList.toggle('preview',d.preview);t.querySelector('.tab-name').textContent=d.name;});
 }
 updateStatus();
}
function updateStatus(){
 if(state.scene==='settings'){el('status-left').innerHTML='设置已保存';el('status-right').innerHTML='<button id="reset-settings">恢复建议默认值</button>';el('reset-settings').onclick=resetSettings;return;}
 const g=currentGroup(),d=docById(g.active);
 if(!d){el('status-left').textContent='就绪';el('status-right').innerHTML='';return;}
 const ta=activeTextarea(),selection=ta?[ta.selectionStart,ta.selectionEnd]:g.selections[d.id]||[0,0];
 const before=d.source.slice(0,selection[0]),line=before.split('\n').length,col=before.length-before.lastIndexOf('\n');
 let left=state.multi&&state.multi.ranges.length>1?state.multi.ranges.length+' 个选区':state.groups.length>1?state.groups.length+' 个区域':isDirty(d)?'未保存修改':'就绪';
 if(d.demoLarge)left='大文件模式 · 场景演示';
 el('status-left').innerHTML='<span class="status-dot"></span><span>'+left+'</span>';
 el('status-right').innerHTML=(groupMode(g,d)==='source'?'<button id="status-position">行 '+line+'，列 '+col+(selection[1]>selection[0]?' · 已选 '+(selection[1]-selection[0]):'')+'</button><button id="status-indent">空格: '+Number(state.settings['editor.tabSize']||4)+'</button>':'')+'<button id="status-encoding">'+d.encoding+'</button><button id="status-eol">'+d.eol+'</button><button id="status-language">'+languageLabel(d.language)+(d.language==='markdown'?' · '+(groupMode(g,d)==='wysiwyg'?'所见即所得':'源码'):'')+'</button>';
 if(el('status-position'))el('status-position').onclick=goToLine;
 if(el('status-indent'))el('status-indent').onclick=function(e){showMenu(e.currentTarget,[[2,4,8].map(function(n){return['缩进: '+n,'',function(){updateSetting('editor.tabSize',n);}];})].flat());};
 el('status-encoding').onclick=function(){showEncodingDialog();};
 el('status-eol').onclick=function(e){showMenu(e.currentTarget,['CRLF','LF'].map(function(value){return[value,value===d.eol?'✓':'',function(){d.eol=value;schedulePersist();updateStatus();toast('保存换行设为 '+value+'；下载时使用该换行。');}];}));};
 el('status-language').onclick=function(e){showMenu(e.currentTarget,['plaintext','markdown','json','javascript','python','rust','html','css'].map(function(lang){return[languageLabel(lang),'',function(){d.language=lang;rememberView();renderWorkspace();updateChrome();}];}));};
 el('app').querySelector('.statusbar').style.display=state.settings['workbench.showStatusBar']===false?'none':'flex';
}
function activeTextarea(){return document.querySelector('.editor-group[data-group="'+state.activeGroup+'"] .source-input');}
function activateDoc(groupId,docId){
 rememberView();state.activeGroup=groupId;const g=currentGroup();g.active=docId;state.find=null;state.multi=null;renderWorkspace();renderSidebar();schedulePersist();
}
function openDocument(docId,preview){
 if(state.scene==='settings')state.scene='main';
 const g=currentGroup(),d=docById(docId);if(!d)return;
 rememberView();
 if(!g.tabs.includes(docId)){
  const old=g.tabs.find(function(id){const x=docById(id);return x&&x.preview&&!isDirty(x)&&!x.pinned;});
  if(preview&&state.settings['workbench.previewTabs']!==false&&old){g.tabs.splice(g.tabs.indexOf(old),1,docId);d.preview=true;}
  else{g.tabs.push(docId);d.preview=!!preview&&state.settings['workbench.previewTabs']!==false;}
 }
 g.active=docId;state.find=null;state.multi=null;renderAll();
}
function newDocument(){
 const number=state.docs.filter(function(d){return d.untitled;}).length+1,id='new-'+Date.now();
 const d=makeDocument(id,'未命名-'+number,'','',true);d.encoding='UTF-8';d.eol=state.settings['files.defaultEol']||'CRLF';
 state.docs.push(d);openDocument(id,false);const ta=activeTextarea();if(ta)ta.focus();toast('已新建临时文本；可直接输入。');
}
function closeTab(groupId,docId,force){
 const g=state.groups.find(function(x){return x.id===groupId;}),d=docById(docId);if(!g||!d)return;
 const otherViews=state.groups.some(function(x){return x.id!==groupId&&x.tabs.includes(docId);});
 if(isDirty(d)&&!otherViews&&!force){
  showDialog('关闭前保留修改？','<p>'+esc(d.name)+' 有未保存内容。这是最后一个打开的视图；“下载并关闭”会另存一个 UTF-8 副本。</p>',[
   ['取消',closeDialog],['放弃修改并关闭',function(){d.source=d.savedSource;closeDialog();closeTab(groupId,docId,true);},'danger'],
   ['下载并关闭',function(){downloadDocument(d);closeDialog();closeTab(groupId,docId,true);},'primary']
  ]);return;
 }
 const ix=g.tabs.indexOf(docId);g.tabs=g.tabs.filter(function(x){return x!==docId;});if(g.active===docId)g.active=g.tabs[Math.max(0,ix-1)]||g.tabs[0]||null;
 if(!g.tabs.length&&state.groups.length>1){state.groups=state.groups.filter(function(x){return x.id!==g.id;});state.activeGroup=state.groups[0].id;}
 state.find=null;state.multi=null;renderAll();schedulePersist();
}
function closeGroup(id,force){
 const g=state.groups.find(function(x){return x.id===id;});if(!g)return;
 if(state.groups.length===1){toast('最后一个编辑区域保留；可以关闭其中的标签。');return;}
 const dirty=g.tabs.map(docById).filter(function(d){return d&&isDirty(d)&&!state.groups.some(function(x){return x.id!==id&&x.tabs.includes(d.id);});});
 if(dirty.length&&!force){showDialog('关闭编辑区域前保留修改？','<p>以下文档的最后一个视图将在此区域关闭：</p><ul>'+dirty.map(function(d){return'<li>'+esc(d.name)+'</li>';}).join('')+'</ul>',[['取消',closeDialog],['放弃修改并关闭',function(){dirty.forEach(function(d){d.source=d.savedSource;});closeDialog();closeGroup(id,true);},'danger'],['下载副本并关闭',function(){dirty.forEach(function(d){downloadDocument(d);});closeDialog();closeGroup(id,true);},'primary']]);return;}
 state.groups=state.groups.filter(function(x){return x.id!==id;});state.activeGroup=state.groups[0].id;state.find=null;renderAll();schedulePersist();
}
function togglePin(){const d=currentDoc();if(!d)return;d.pinned=!d.pinned;d.preview=false;updateChrome();schedulePersist();}
function tabMenu(e,docId,groupId){
 const d=docById(docId);
 showMenuAt(e.clientX,e.clientY,[
  [d.pinned?'取消固定':'固定标签','',function(){d.pinned=!d.pinned;d.preview=false;updateChrome();}],
  ['拆分显示此文档','',function(){activateDoc(groupId,docId);splitCurrent();}],
  ['移动到新窗口','',function(){activateDoc(groupId,docId);newWindow();}],
  null,
  ['关闭标签','Ctrl+W',function(){closeTab(groupId,docId);}],
  ['关闭其他已保存标签','',function(){const g=state.groups.find(function(x){return x.id===groupId;});g.tabs=g.tabs.filter(function(id){const x=docById(id);return id===docId||isDirty(x)||x.pinned;});g.active=docId;renderAll();}],
  ['复制文件路径','',function(){copyText(d.path||d.name);}]
 ]);
}
function moveTab(fromId,toId,docId){
 if(fromId===toId)return;
 const from=state.groups.find(function(x){return x.id===fromId;}),to=state.groups.find(function(x){return x.id===toId;});if(!from||!to)return;
 if(!to.tabs.includes(docId))to.tabs.push(docId);to.active=docId;
 from.tabs=from.tabs.filter(function(x){return x!==docId;});if(from.active===docId)from.active=from.tabs[0];
 if(!from.tabs.length)state.groups=state.groups.filter(function(x){return x.id!==fromId;});
 state.activeGroup=toId;state.find=null;renderAll();toast('标签已移动；文档内容和撤销历史保持共享。');
}
function splitCurrent(){
 if(state.scene==='settings')return;
 if(state.groups.length>=3){toast('原型提供三个交互区域；正式产品的组数与边界见 PRD。');return;}
 rememberView();const current=currentGroup(),d=currentDoc();if(!d)return;
 const g=makeGroup('g'+Date.now(),[d.id],d.id);g.modes[d.id]=groupMode(current,d);
 state.groups.push(g);state.scene='split';state.activeGroup=g.id;state.find=null;renderAll();
}
function switchScene(scene){
 if(state.composing){toast('请先完成当前输入法组合。');return;}
 rememberView();state.find=null;state.multi=null;state.zen=false;document.body.classList.remove('zen');
 if(scene==='main'){
  state.groups=[makeGroup('g1',['todo','project','snippet','scratch'].filter(function(id){return docById(id);}),docById('todo')?'todo':state.docs[0].id)];
  if(!state.groups[0].tabs.length)state.groups[0]=makeGroup('g1',[state.docs[0].id]);
  state.activeGroup='g1';state.sidebar='files';state.scene='main';
 }else if(scene==='split'){
  const id1=docById('project')?'project':state.docs[0].id,id2=docById('snippet')?'snippet':id1,id3=docById('todo')?'todo':id1;
  state.groups=[makeGroup('g1',[id1]),makeGroup('g2',[id2]),makeGroup('g3',[id3])];state.groups[0].modes[id1]='wysiwyg';state.activeGroup='g1';state.sidebar='files';state.scene='split';
 }else if(scene==='settings'){state.scene='settings';state.settingsCategory='常用';}
 state.sidebarVisible=true;renderAll();
}
function toggleMarkdown(groupId){
 if(state.composing){toast('请先完成输入法组合，再切换编辑模式。');return;}
 const g=state.groups.find(function(x){return x.id===(groupId||state.activeGroup);});if(!g)return;
 const d=docById(g.active);if(!d||d.language!=='markdown'){toast('此命令适用于 Markdown 文档。');return;}
 rememberView();const next=groupMode(g,d)==='source'?'wysiwyg':'source';setMode(g,d,next);state.activeGroup=g.id;state.multi=null;state.find=null;renderWorkspace();updateStatus();schedulePersist();
 toast(next==='source'?'已切换到 Markdown 源码；切换本身未改写文档。':'已返回所见即所得编辑；文档撤销历史继续保留。');
}
function forceSource(reason){
 const g=currentGroup(),d=currentDoc();if(!d)return false;
 if(groupMode(g,d)==='wysiwyg'){rememberView();setMode(g,d,'source');renderWorkspace();toast(reason||'已切换到源码编辑。');return true;}
 return false;
}
function toggleSidebar(){state.sidebarVisible=!state.sidebarVisible;renderSidebar();}
function toggleZen(){
 if(state.composing){toast('请先完成当前输入法组合。');return;}
 if(state.scene==='settings'){toast('请先打开一个文档，再进入 Zen 模式。');return;}
 state.zen=!state.zen;document.body.classList.toggle('zen',state.zen);
 if(state.zen){state.zenGroup=state.activeGroup;document.querySelectorAll('.editor-group').forEach(function(g){if(g.dataset.group!==state.activeGroup)g.style.display='none';});document.querySelectorAll('.split-handle').forEach(function(x){x.style.display='none';});document.querySelectorAll('.split-left,.split-right').forEach(function(x){if(!x.querySelector('.editor-group:not([style*="display: none"])'))x.style.display='none';else x.style.width='100%';});toast('已进入 Zen；按两次 Esc 或点击右上角退出。');}
 else renderWorkspace();
}
function bindSplitHandles(){
 document.querySelectorAll('.split-handle:not(.horizontal)').forEach(function(h){
  h.onpointerdown=function(e){e.preventDefault();h.setPointerCapture(e.pointerId);const parent=h.parentElement,left=parent.querySelector('.split-left'),box=parent.getBoundingClientRect();h.onpointermove=function(ev){left.style.width=Math.max(25,Math.min(78,(ev.clientX-box.left)/box.width*100))+'%';};h.onpointerup=function(){h.onpointermove=null;};};
 });
 document.querySelectorAll('.split-handle.horizontal').forEach(function(h){
  h.onpointerdown=function(e){e.preventDefault();h.setPointerCapture(e.pointerId);const top=h.previousElementSibling,bottom=h.nextElementSibling,box=h.parentElement.getBoundingClientRect();h.onpointermove=function(ev){const p=Math.max(.2,Math.min(.8,(ev.clientY-box.top)/box.height));top.style.flex=String(p);bottom.style.flex=String(1-p);};h.onpointerup=function(){h.onpointermove=null;};};
 });
}
function replaceSelection(text,force){
 forceSource('该文本编辑操作在源码模式执行。');const ta=activeTextarea(),d=currentDoc(),g=currentGroup();if(!ta||!d)return;
 if(state.multi&&state.multi.ranges.length>1){applyMulti(text);return;}
 const start=ta.selectionStart,end=ta.selectionEnd,next=d.source.slice(0,start)+text+d.source.slice(end);
 if(editDocument(d,next,'replace-selection',force)){ta.value=next;ta.focus();ta.setSelectionRange(start+text.length,start+text.length);updateSourceView(ta,d,g);updateOtherViews(d.id,g.id);if(state.find)runFind();}
 updateStatus();
}
function selectedWord(ta){
 let a=ta.selectionStart,b=ta.selectionEnd;if(a!==b)return[a,b];
 const s=ta.value;while(a>0&&/[\p{L}\p{N}_]/u.test(s[a-1]))a--;while(b<s.length&&/[\p{L}\p{N}_]/u.test(s[b]))b++;return[a,b];
}
function addNextSelection(all){
 forceSource('多光标编辑在源码模式进行。');const ta=activeTextarea(),d=currentDoc(),g=currentGroup();if(!ta)return;
 let ranges=state.multi&&state.multi.group===g.id?state.multi.ranges.slice():[selectedWord(ta)];
 let needle=state.multi&&state.multi.group===g.id?state.multi.needle:ta.value.slice(ranges[0][0],ranges[0][1]);if(!needle){toast('请先选中要批量编辑的文字。');return;}
 const found=[];let at=0;while((at=d.source.indexOf(needle,at))>=0){found.push([at,at+needle.length]);at+=Math.max(1,needle.length);if(found.length>=1000)break;}
 if(all)ranges=found;
 else{
  const next=found.find(function(r){return r[0]>=ranges[ranges.length-1][1]&&!ranges.some(function(x){return x[0]===r[0];});})||found.find(function(r){return!ranges.some(function(x){return x[0]===r[0];});});
  if(next)ranges.push(next);
 }
 state.multi={group:g.id,doc:d.id,ranges:ranges.sort(function(a,b){return a[0]-b[0];}),needle};
 const last=ranges[ranges.length-1];ta.setSelectionRange(last[0],last[1]);ta.focus();updateSourceView(ta,d,g);updateStatus();toast(ranges.length+' 个选区；输入文字将同时替换。');
}
function addCursorAt(g,offset){
 const ta=activeTextarea();if(!ta)return;
 const old=state.multi&&state.multi.group===g.id?state.multi.ranges:((g.selections[g.active]&&g.selections[g.active][0]!==offset)?[g.selections[g.active]]:[]);
 const ranges=old.concat([[offset,offset]]).filter(function(r,i,a){return a.findIndex(function(x){return x[0]===r[0]&&x[1]===r[1];})===i;}).sort(function(a,b){return a[0]-b[0];});
 state.multi={group:g.id,doc:g.active,ranges,needle:''};updateSourceView(ta,currentDoc(),g);updateStatus();
}
function applyMulti(text,backspace){
 const m=state.multi,d=currentDoc(),g=currentGroup();if(!m||!d||d.readOnly)return;
 const ranges=m.ranges.map(function(r){return backspace&&r[0]===r[1]?[Math.max(0,r[0]-1),r[1]]:r.slice();});
 let next=d.source;for(let i=ranges.length-1;i>=0;i--)next=next.slice(0,ranges[i][0])+text+next.slice(ranges[i][1]);
 if(!editDocument(d,next,'multi',true))return;
 let delta=0;m.ranges=ranges.map(function(r){const end=r[0]+delta+text.length;delta+=text.length-(r[1]-r[0]);return[end,end];});m.needle='';
 const ta=activeTextarea();ta.value=next;const last=m.ranges[m.ranges.length-1];ta.setSelectionRange(last[0],last[1]);updateSourceView(ta,d,g);updateOtherViews(d.id,g.id);updateStatus();
}
function duplicateLine(){
 forceSource();const ta=activeTextarea(),d=currentDoc(),g=currentGroup();if(!ta)return;
 const at=ta.selectionStart,a=d.source.lastIndexOf('\n',Math.max(0,at-1))+1,e=d.source.indexOf('\n',at),b=e<0?d.source.length:e;
 const line=d.source.slice(a,b),next=d.source.slice(0,b)+'\n'+line+d.source.slice(b);
 editDocument(d,next,'duplicate-line',true);ta.value=next;ta.setSelectionRange(b+1,b+1);updateSourceView(ta,d,g);updateOtherViews(d.id,g.id);
}
function formatRich(tag){
 const d=currentDoc(),g=currentGroup();if(!d||groupMode(g,d)!=='wysiwyg'){if(tag==='bold')toggleSidebar();return;}
 const selection=window.getSelection();if(!selection||!selection.rangeCount||selection.isCollapsed){toast('先选中正文文字，再应用格式。');return;}
 const node=selection.anchorNode.nodeType===1?selection.anchorNode:selection.anchorNode.parentElement,block=node.closest('[data-block]');
 if(!block||!block.closest('.editor-group[data-group="'+g.id+'"]'))return;
 document.execCommand(tag,false);commitRichBlock(block,d);
}

function searchCore(payload){
 const text=payload.text,q=payload.query,o=payload.options||{},matches=[];
 if(!q)return{matches,error:null};
 function word(c){return!!c&&/[\p{L}\p{M}\p{N}_]/u.test(c);}
 function boundary(a,b){return!word(text.slice(0,a).match(/.$/u)?.[0])&&!word(text.slice(b).match(/^./u)?.[0]);}
 let re;
 try{const source=o.regex?q:q.replace(/[.*+?^$()|[\]\\]/g,'\\$&');re=new RegExp(source,'gu'+(o.case?'':'i')+'m');}
 catch(e){return{matches,error:'表达式无效：'+e.message};}
 const startTime=Date.now();let m;
 while((m=re.exec(text))){
  if(!o.word||boundary(m.index,m.index+m[0].length))matches.push({start:m.index,end:m.index+m[0].length,text:m[0],captures:Array.from(m).slice(1),groups:m.groups||{}});
  if(matches.length>=20000)return{matches,error:null,truncated:true};
  if(m[0]===''){const cp=text.codePointAt(re.lastIndex);re.lastIndex+=cp>65535?2:1;}
  if(Date.now()-startTime>350)return{matches,error:'搜索已达到原型时间预算，请缩小范围。'};
 }
 return{matches,error:null,truncated:false};
}
let searchWorker=null,searchTimeout=null,searchEpoch=0,lastSearchText='';
function richTextIndex(node){
 const segments=[];let text='',lastBlock=null;
 const walker=document.createTreeWalker(node,NodeFilter.SHOW_TEXT,{acceptNode:function(n){return n.parentElement.closest('.raw-label,button')?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT;}});
 let n;while((n=walker.nextNode())){
  const block=n.parentElement.closest('[data-block]');if(lastBlock&&block!==lastBlock)text+='\n';
  const start=text.length;text+=n.textContent;segments.push({node:n,start,end:text.length});lastBlock=block;
 }
 return{text,segments};
}
function openFind(replace){
 if(state.scene==='settings'){el('settings-query')?.focus();return;}
 const g=currentGroup(),d=currentDoc();if(!d)return;
 if(replace)forceSource('替换在 Markdown 源码中执行；查找范围已切为“源码”。');
 const old=state.find,ta=activeTextarea(),selected=ta&&ta.selectionStart!==ta.selectionEnd?ta.value.slice(ta.selectionStart,ta.selectionEnd):'';
 state.find={group:g.id,doc:d.id,query:old&&old.doc===d.id?old.query:selected,case:old?old.case:!!state.settings['search.matchCase'],word:old?old.word:!!state.settings['search.wholeWord'],regex:old?old.regex:!!state.settings['search.regex'],replace:!!replace,replacement:old?old.replacement||'':''};
 if(state.find.regex)forceSource('正则搜索在源码中执行；查找范围已切为“源码”。');
 rememberView();state.searchResult={matches:[],current:0,error:null};renderWorkspace();const field=document.querySelector('.find-query');if(field){field.focus();field.select();}runFind();
}
function bindFind(node,g,d){
 const widget=node.querySelector('.find-widget');if(!widget)return;
 widget.querySelector('.find-query').oninput=function(e){state.find.query=e.target.value;state.searchResult.current=0;runFind();};
 widget.querySelector('.find-query').onkeydown=function(e){if(e.key==='Enter'){e.preventDefault();nextMatch(e.shiftKey?-1:1);}if(e.key==='Escape'){e.preventDefault();closeFind();}};
 const rep=widget.querySelector('.replace-query');if(rep)rep.oninput=function(e){state.find.replacement=e.target.value;};
 widget.querySelectorAll('[data-find-option]').forEach(function(b){b.onclick=function(){
  const field=b.dataset.findOption;state.find[field]=!state.find[field];state.searchResult.current=0;
  if(field==='regex'&&state.find.regex&&groupMode(g,d)==='wysiwyg'){rememberView();setMode(g,d,'source');renderWorkspace();toast('正则搜索已切换到源码，匹配计数已重新计算。');}
  else b.classList.toggle('active',state.find[field]);runFind();
 };});
 widget.querySelector('.find-close').onclick=closeFind;
 widget.querySelector('.find-expand').onclick=function(){
  state.find.replace=!state.find.replace;
  if(state.find.replace&&groupMode(g,d)==='wysiwyg'){setMode(g,d,'source');toast('替换范围已切换为 Markdown 源码。');}
  rememberView();renderWorkspace();runFind();document.querySelector(state.find.replace?'.replace-query':'.find-query')?.focus();
 };
 widget.querySelector('.find-prev').onclick=function(){nextMatch(-1);};widget.querySelector('.find-next').onclick=function(){nextMatch(1);};
 widget.querySelector('.replace-one')?.addEventListener('click',function(){replaceMatches(false);});
 widget.querySelector('.replace-all')?.addEventListener('click',function(){replaceMatches(true);});
}
function closeFind(){
 searchEpoch++;if(searchWorker){searchWorker.terminate();searchWorker=null;}clearTimeout(searchTimeout);
 state.find=null;state.searchResult={matches:[],current:0,error:null};
 if(CSS.highlights){CSS.highlights.delete('texzi-search');CSS.highlights.delete('texzi-current');}
 rememberView();renderWorkspace();activeTextarea()?.focus();
}
function runFind(){
 const f=state.find;if(!f)return;
 const g=state.groups.find(function(x){return x.id===f.group;}),d=docById(f.doc);if(!g||!d)return;
 const group=document.querySelector('[data-group="'+g.id+'"]'),rich=group?.querySelector('.rich-document'),index=rich?richTextIndex(rich):null;
 const text=index?index.text:d.source;lastSearchText=text;
 const payload={text,query:f.query,options:{case:f.case,word:f.word,regex:f.regex}},epoch=++searchEpoch;
 if(searchWorker){searchWorker.terminate();searchWorker=null;}clearTimeout(searchTimeout);
 function apply(result){
  if(epoch!==searchEpoch||!state.find)return;
  state.searchResult={matches:result.matches||[],current:Math.min(state.searchResult.current,Math.max(0,(result.matches||[]).length-1)),error:result.error||null,truncated:result.truncated};
  refreshFindVisuals(index);
 }
 if(!f.query){apply({matches:[]});return;}
 if(typeof Worker==='undefined'){if(f.regex){apply({matches:[],error:'此浏览器无法运行后台正则搜索；请使用普通查找。'});return;}apply(searchCore(payload));return;}
 try{
  const blob=new Blob(['onmessage=function(e){postMessage(('+searchCore.toString()+')(e.data));};'],{type:'text/javascript'});
  const url=URL.createObjectURL(blob);searchWorker=new Worker(url);URL.revokeObjectURL(url);
  searchWorker.onmessage=function(e){clearTimeout(searchTimeout);searchWorker?.terminate();searchWorker=null;apply(e.data);};
  searchWorker.onerror=function(){clearTimeout(searchTimeout);searchWorker?.terminate();searchWorker=null;if(!f.regex)apply(searchCore(payload));else apply({matches:[],error:'正则工作线程不可用；未在界面线程执行表达式。'});};
  searchTimeout=setTimeout(function(){searchWorker?.terminate();searchWorker=null;apply({matches:[],error:'表达式执行超过 500 ms，搜索已取消。'});},500);
  searchWorker.postMessage(payload);
 }catch(e){if(!f.regex)apply(searchCore(payload));else apply({matches:[],error:'当前环境不允许后台正则；普通查找仍可使用。'});}
}
function refreshFindVisuals(index){
 const f=state.find;if(!f)return;const g=state.groups.find(function(x){return x.id===f.group;}),d=docById(f.doc),node=document.querySelector('[data-group="'+f.group+'"]');if(!node)return;
 const r=state.searchResult,widget=node.querySelector('.find-widget');
 if(widget){widget.querySelector('.find-result').textContent=r.matches.length?(r.current+1)+' / '+r.matches.length+(r.truncated?'+':''):'无结果';const er=widget.querySelector('.find-error');er.textContent=r.error||'';er.hidden=!r.error;}
 const ta=node.querySelector('.source-input');if(ta)updateSourceView(ta,d,g);
 const rich=node.querySelector('.rich-document');
 if(rich&&CSS.highlights&&typeof Highlight!=='undefined'){
  const ix=index||richTextIndex(rich),ranges=[];
  r.matches.slice(0,3000).forEach(function(m){
   const a=ix.segments.find(function(s){return s.end>m.start&&s.start<=m.start;}),b=ix.segments.find(function(s){return s.end>=m.end&&s.start<m.end;});
   if(!a||!b)return;try{const range=new Range();range.setStart(a.node,m.start-a.start);range.setEnd(b.node,m.end-b.start);ranges.push(range);}catch(e){}
  });
  CSS.highlights.set('texzi-search',new Highlight(...ranges));CSS.highlights.set('texzi-current',new Highlight(...(ranges[r.current]?[ranges[r.current]]:[])));
 }
}
function nextMatch(direction){
 const r=state.searchResult;if(!r.matches.length)return;
 const next=r.current+direction;if(state.settings['search.wrapAround']===false&&(next<0||next>=r.matches.length)){toast('已到匹配结果边界。');return;}
 r.current=(next+r.matches.length)%r.matches.length;refreshFindVisuals();revealMatch();
}
function revealMatch(){
 const f=state.find,r=state.searchResult;if(!f||!r.matches.length)return;
 const m=r.matches[r.current],node=document.querySelector('[data-group="'+f.group+'"]'),ta=node.querySelector('.source-input');
 if(ta){ta.setSelectionRange(m.start,m.end);const row=ta.value.slice(0,m.start).split('\n').length-1;ta.scrollTop=Math.max(0,row*Number(state.settings['editor.fontSize'])*Number(state.settings['editor.lineHeight'])-ta.clientHeight/3);syncSourceScroll(ta);}
 else{const rich=node.querySelector('.rich-document'),ix=richTextIndex(rich),s=ix.segments.find(function(x){return x.end>m.start;});if(s)s.node.parentElement.scrollIntoView({block:'center',behavior:'auto'});}
}
function expandReplacement(template,m){
 return template.replace(/\$\$|\$&|\$0|\$\d{1,2}|\$\{[^}]+\}/g,function(token){
  if(token==='$$')return'$';if(token==='$&'||token==='$0')return m.text;
  if(token.startsWith('$'+'{')){const name=token.slice(2,-1);if(!Object.prototype.hasOwnProperty.call(m.groups,name))throw new Error('不存在命名捕获组：'+name);return m.groups[name]||'';}
  const index=Number(token.slice(1))-1;if(index<0||index>=m.captures.length)throw new Error('不存在捕获组：'+token);return m.captures[index]||'';
 });
}
function replaceMatches(all){
 const f=state.find;if(!f)return;const d=docById(f.doc),g=state.groups.find(function(x){return x.id===f.group;});
 if(d.readOnly){toast('只读文件不能替换。');return;}if(!state.searchResult.matches.length)return;
 if(state.searchResult.error||state.searchResult.truncated){toast('搜索未完整完成，不能执行替换；请缩小范围后重试。');return;}
 if(lastSearchText!==d.source){runFind();toast('内容已变化，已刷新结果，请再次执行替换。');return;}
 const list=all?state.searchResult.matches:[state.searchResult.matches[state.searchResult.current]];
 let next=d.source;try{for(let i=list.length-1;i>=0;i--){const m=list[i],text=f.regex?expandReplacement(f.replacement||'',m):f.replacement||'';next=next.slice(0,m.start)+text+next.slice(m.end);}}catch(error){toast('替换未执行：'+error.message);return;}
 editDocument(d,next,'replace-all',true);const ta=activeTextarea();if(ta){ta.value=d.source;updateSourceView(ta,d,g);}updateOtherViews(d.id,g.id);runFind();toast('已替换 '+list.length+' 处；一次撤销可恢复。');
}
const COMMON_SETTINGS=['appearance.theme','editor.fontFamily','editor.fontSize','editor.wordWrap.text','editor.lineNumbers','markdown.defaultView','files.autoSave','session.restoreOnStartup'];
function settingDisplayName(s){return{'editor.fontSize':'字号','editor.wordWrap.text':'自动换行','markdown.defaultView':'Markdown 默认模式','session.restoreOnStartup':'恢复上次会话'}[s.id]||s.name;}
function settingControl(s){
 const value=state.settings[s.id]===undefined?s.default:state.settings[s.id],id=esc(s.id);
 if(s.type==='boolean')return'<button class="switch '+(value?'on':'')+'" role="switch" aria-checked="'+!!value+'" aria-label="'+esc(s.name)+'" data-setting="'+id+'" data-kind="boolean"></button>';
 if(s.options&&s.options.length)return'<select data-setting="'+id+'" aria-label="'+esc(s.name)+'">'+s.options.map(function(o){const v=typeof o==='object'?o.value:o,l=typeof o==='object'?o.label:o;return'<option value="'+esc(v)+'" '+(String(value)===String(v)?'selected':'')+'>'+esc(l)+'</option>';}).join('')+'</select>';
 if(s.type==='number'||s.type==='integer')return'<input type="number" data-setting="'+id+'" aria-label="'+esc(s.name)+'" value="'+esc(value)+'" min="'+(s.min===undefined?0:s.min)+'" max="'+(s.max===undefined?100000:s.max)+'" step="'+(s.step||1)+'">'+(s.unit?'<span style="font-size:12px;color:var(--muted)">'+esc(s.unit)+'</span>':'');
 return'<input type="text" data-setting="'+id+'" aria-label="'+esc(s.name)+'" value="'+esc(Array.isArray(value)?value.join(', '):value)+'">';
}
function renderSettings(){
 const search=state.settingsSearch.toLowerCase(),category=state.settingsCategory;
 let list=SETTING_CATALOG.settings.filter(function(s){
  if(search)return(s.name+' '+s.id+' '+s.description).toLowerCase().includes(search);
  if(category==='常用')return COMMON_SETTINGS.includes(s.id);
  if(category==='文件与恢复')return['文件','恢复','文件与恢复'].includes(s.category);
  if(category==='编辑器')return['编辑器','查找','大文件'].includes(s.category);
  if(category==='窗口与标签')return['窗口与标签','Zen'].includes(s.category);
  return s.category===category;
 });
 if(category==='常用'&&!search)list.sort(function(a,b){return COMMON_SETTINGS.indexOf(a.id)-COMMON_SETTINGS.indexOf(b.id);});
 const title=search?'搜索设置':category==='常用'?'常用设置':category;
 let body='';
 if(category==='快捷键'&&!search){
  body='<div class="review-callout">Windows 默认快捷键对照 VS Code '+esc(COMMAND_CATALOG.baseline?.vscodeVersion||'1.146.0')+'。原型仅绑定常用审阅操作，浏览器可能保留部分系统快捷键；完整实现范围见交付文档。</div><table class="shortcut-table"><thead><tr><th>命令</th><th>默认快捷键</th><th>上下文</th></tr></thead><tbody>'+COMMAND_CATALOG.commands.map(function(c){return'<tr><td>'+esc(c.name)+'</td><td>'+(c.defaultKeys.length?c.defaultKeys.map(function(k){return'<span class="shortcut-key">'+esc(k)+'</span>';}).join(' '):'未绑定')+'</td><td>'+esc(c.context)+'</td></tr>';}).join('')+'</tbody></table>';
 }else{
  const groups=[];
  list.forEach(function(s){let name=s.subcategory||s.category;if(category==='常用'){name=['appearance.theme','editor.fontFamily','editor.fontSize'].includes(s.id)?'外观与文字':['files.autoSave','session.restoreOnStartup'].includes(s.id)?'文件与恢复':'编辑与 Markdown';}
   let group=groups.find(function(x){return x.name===name;});if(!group){group={name,items:[]};groups.push(group);}group.items.push(s);
  });
  body=groups.map(function(group){return'<section class="settings-group"><h2>'+esc(group.name)+'</h2>'+group.items.map(function(s){
   const desc=category==='常用'?({'files.autoSave':'自动写入原文件','session.restoreOnStartup':'包含未保存的临时文本'}[s.id]||''):s.description;
   return'<div class="setting-row"><div class="setting-label"><div class="setting-name">'+esc(category==='常用'?settingDisplayName(s):s.name)+'</div>'+(desc?'<div class="setting-desc">'+esc(desc)+'</div>':'')+'</div><div class="setting-control">'+settingControl(s)+'</div></div>';
  }).join('')+'</section>';}).join('');
  if(!list.length)body='<div class="settings-empty">没有找到匹配设置。试试“字体”“恢复”或设置 ID。</div>';
 }
 el('workspace').innerHTML='<div class="settings-page"><div class="tabbar"><div class="tab-list"><div class="tab" data-return-doc="todo"><span class="tab-name">今日待办.txt</span></div><div class="tab" data-return-doc="project"><span class="tab-name">项目笔记.md</span></div><div class="tab active" style="border-bottom:2px solid var(--accent)"><span class="tab-name">设置</span><button class="tab-close settings-close" aria-label="关闭设置">×</button></div></div></div><div class="settings-scroll"><div class="settings-content"><div class="settings-top"><h1>'+esc(title)+'</h1><input id="settings-query" placeholder="搜索设置" aria-label="搜索设置" value="'+esc(state.settingsSearch)+'"></div>'+body+'</div></div></div>';
 el('workspace').querySelectorAll('[data-setting]').forEach(function(control){
  if(control.dataset.kind==='boolean'){control.onclick=function(){const next=!state.settings[control.dataset.setting];control.classList.toggle('on',next);control.setAttribute('aria-checked',String(next));updateSetting(control.dataset.setting,next,false);};}
  else control.onchange=function(){const s=SETTING_CATALOG.settings.find(function(x){return x.id===control.dataset.setting;});let v=control.value;if(s.type==='number'||s.type==='integer')v=Math.max(s.min||0,Math.min(s.max||100000,Number(v)));if(s.type==='array')v=v.split(',').map(function(x){return x.trim();}).filter(Boolean);updateSetting(s.id,v,false);};
 });
 el('settings-query').oninput=function(e){const pos=e.target.selectionStart;state.settingsSearch=e.target.value;renderSettings();el('settings-query').focus();el('settings-query').setSelectionRange(pos,pos);};
 el('workspace').querySelectorAll('[data-return-doc]').forEach(function(t){t.onclick=function(){state.scene=state.groups.length>1?'split':'main';openDocument(t.dataset.returnDoc,false);};});
 el('workspace').querySelector('.settings-close').onclick=function(){state.scene=state.groups.length>1?'split':'main';renderAll();};
 updateStatus();
}
function updateSetting(id,value,rerender){
 state.settings[id]=value;applySettings();schedulePersist();
 if(rerender!==false){rememberView();renderAll();}else updateStatus();
 const live=['appearance.theme','appearance.interfaceFontSize','editor.fontFamily','editor.fontSize','editor.lineHeight','editor.lineNumbers','editor.tabSize','editor.wordWrap.text','editor.wordWrap.markdown','editor.wordWrap.code','markdown.defaultView','markdown.fontSize','markdown.lineHeight','markdown.contentWidth','files.autoSave','files.autoSaveDelay','session.restoreOnStartup','session.restoreUnsaved','workbench.previewTabs','workbench.showStatusBar'];
 if(!live.includes(id))toast('设置值已保存；此项的原生行为在当前原型中仅作配置演示。');
}
function resetSettings(){showDialog('恢复建议默认值','<p>将当前原型中的设置恢复为交付包建议默认值，不修改文档内容。</p>',[['取消',closeDialog],['恢复默认',function(){state.settings=Object.assign({},defaultSettings);closeDialog();applySettings();renderAll();persist();},'primary']]);}
function showMenu(anchor,items){const r=anchor.getBoundingClientRect();showMenuAt(r.left,r.bottom+3,items);}
function showMenuAt(x,y,items){
 closeMenu();const root=el('menu-root');root.innerHTML='<div class="menu" role="menu">'+items.map(function(item,i){if(!item)return'<div class="menu-separator"></div>';return'<button class="menu-item" role="menuitem" data-menu-index="'+i+'" '+(item[3]?'disabled':'')+'><span>'+esc(item[0])+'</span><kbd>'+esc(item[1]||'')+'</kbd></button>';}).join('')+'</div>';
 const menu=root.firstChild;menu.style.left=Math.max(5,Math.min(x,window.innerWidth-menu.offsetWidth-8))+'px';menu.style.top=Math.max(5,Math.min(y,window.innerHeight-menu.offsetHeight-8))+'px';
 menu.querySelectorAll('[data-menu-index]').forEach(function(b){b.onclick=function(){const item=items[Number(b.dataset.menuIndex)];closeMenu();if(item&&item[2])item[2]();};});
}
function closeMenu(){el('menu-root').innerHTML='';document.querySelectorAll('.menubar-button').forEach(function(b){b.classList.remove('open');});}
function showDialog(title,html,actions,wide){
 closeMenu();el('dialog-root').innerHTML='<div class="backdrop"><section class="dialog '+(wide?'wide':'')+'" role="dialog" aria-modal="true" aria-label="'+esc(title)+'"><button class="dialog-close" aria-label="关闭对话框">×</button><h2>'+esc(title)+'</h2>'+html+'<div class="dialog-actions">'+actions.map(function(a,i){return'<button data-dialog-action="'+i+'" class="'+(a[2]||'')+'">'+esc(a[0])+'</button>';}).join('')+'</div></section></div>';
 el('dialog-root').querySelector('.dialog-close').onclick=closeDialog;
 el('dialog-root').querySelectorAll('[data-dialog-action]').forEach(function(b){b.onclick=actions[Number(b.dataset.dialogAction)][1];});
 el('dialog-root').querySelector('input,button.primary,.dialog-actions button')?.focus();
}
function closeDialog(){el('dialog-root').innerHTML='';}
function buildMenus(){
 const menus={
  '文件':function(){return[['新建文本','Ctrl+N',newDocument],['打开文件…','Ctrl+O',function(){el('file-input').click();}],['打开文件夹…','Ctrl+K Ctrl+O',function(){el('folder-input').click();}],null,['保存（下载文件）','Ctrl+S',function(){saveCurrent(false);}],['另存为…','Ctrl+Shift+S',function(){saveCurrent(true);}],['新建窗口','Ctrl+Shift+N',newWindow],null,['关闭标签','Ctrl+W',function(){closeTab(state.activeGroup,currentGroup().active);}],['恢复上次关闭的标签','Ctrl+Shift+T',function(){const d=state.docs.find(function(x){return!state.groups.some(function(g){return g.tabs.includes(x.id);});});if(d)openDocument(d.id,false);else toast('没有已关闭标签。');}]];},
  '编辑':function(){return[['撤销','Ctrl+Z',function(){historyAction(false);}],['重做','Ctrl+Y',function(){historyAction(true);}],null,['剪切','Ctrl+X',function(){copyOrCut(true);}],['复制','Ctrl+C',function(){copyOrCut(false);}],['粘贴…','Ctrl+V',pasteFromMenu],null,['查找','Ctrl+F',function(){openFind(false);}],['替换','Ctrl+H',function(){openFind(true);}],null,['设置','Ctrl+,',function(){switchScene('settings');}]];},
  '选择':function(){return[['全选','Ctrl+A',selectAll],['选择下一个相同内容','Ctrl+D',function(){addNextSelection(false);}],['选择全部相同内容','Ctrl+Shift+L',function(){addNextSelection(true);}],['复制当前行到下方','Shift+Alt+↓',duplicateLine],null,['清除额外选区','Esc',clearMulti]];},
  '查看':function(){return[['命令面板','Ctrl+Shift+P',function(){openPalette(true);}],['显示 / 隐藏侧栏','Ctrl+K Ctrl+B',toggleSidebar],['文件夹搜索','Ctrl+Shift+F',function(){state.scene=state.groups.length>1?'split':'main';state.sidebar='search';state.sidebarVisible=true;renderAll();el('workspace-query')?.focus();}],null,['拆分编辑区域','Ctrl+\\',splitCurrent],['Markdown：切换源码','>_',function(){toggleMarkdown();}],['Zen 模式','Ctrl+K Z',toggleZen],null,['浅色主题','',function(){updateSetting('appearance.theme','light');}],['深色主题','',function(){updateSetting('appearance.theme','dark');}]];},
  '转到':function(){return[['打开文件或命令','Ctrl+P',function(){openPalette(false);}],['转到行…','Ctrl+G',goToLine],['下一个标签','Ctrl+Tab',function(){cycleTab(1);}],['上一个标签','Ctrl+Shift+Tab',function(){cycleTab(-1);}],null,['编辑组 1','Ctrl+1',function(){focusGroup(0);}],['编辑组 2','Ctrl+2',function(){focusGroup(1);}],['编辑组 3','Ctrl+3',function(){focusGroup(2);}]];},
  '帮助':function(){return[['原型功能说明','',showPrototypeHelp],['审阅建议路径','',showReviewGuide],['关于 texzi','',function(){showDialog('texzi','<p>text 与“字”结合。为日常文字、Markdown 和代码片段设计的 Windows 原生文本编辑器。</p><p>当前为设计交付包 v1.0 的浏览器交互原型；最终产品采用 NSH。</p>',[['关闭',closeDialog,'primary']]);}],null,['重置演示数据…','',resetDemo]];}
 };
 el('menubar').innerHTML=Object.keys(menus).map(function(name){return'<button class="menubar-button" aria-haspopup="menu">'+name+'</button>';}).join('');
 el('menubar').querySelectorAll('button').forEach(function(b){b.onclick=function(){showMenu(b,menus[b.textContent]());b.classList.add('open');};});
}
function renderAll(){
 applySettings();renderSidebar();renderWorkspace();
 document.querySelectorAll('[data-scene]').forEach(function(b){b.classList.toggle('selected',b.dataset.scene===state.scene);});
 updateChrome();schedulePersist();
}
function downloadDocument(d,name){
 const filename=name||d.name+(d.untitled&&!/\.\w+$/.test(d.name)?'.txt':'');
 const normalized=d.source.replace(/\r\n|\r/g,'\n'),content=(d.bom?'\uFEFF':'')+(d.eol==='CRLF'?normalized.replace(/\n/g,'\r\n'):normalized);
 const blob=new Blob([content],{type:'text/plain;charset=utf-8'}),url=URL.createObjectURL(blob),a=document.createElement('a');
 a.href=url;a.download=filename;document.body.appendChild(a);a.click();a.remove();setTimeout(function(){URL.revokeObjectURL(url);},1500);
 d.savedSource=d.source;d.lastKind='';schedulePersist();updateChrome();
}
function saveCurrent(as){
 const d=currentDoc();if(!d)return;
 if(d.banner==='save-error'){toast('保存失败场景仍生效；使用横幅“另存为”继续。');return;}
 if(as||d.untitled){
  showDialog('另存为','<p>浏览器原型会下载 UTF-8 文本副本。原生产品的写盘、编码转换与冲突处理见技术方案。</p><label class="field">文件名<input id="save-name" value="'+esc(d.name+(d.untitled&&!/\.\w+$/.test(d.name)?'.txt':''))+'"></label>',[['取消',closeDialog],['下载文件',function(){const name=el('save-name').value.trim();if(!name||/[<>:"/\\|?*]/.test(name)){toast('请输入有效的 Windows 文件名。');return;}downloadDocument(d,name);d.name=name;d.untitled=false;d.path='已下载/'+name;d.language=languageFor(name);closeDialog();renderAll();},'primary']]);
 }else{downloadDocument(d);toast('已下载 '+d.name+'；本机原文件未被覆盖。');}
}
async function loadFiles(files,folder){
 const input=Array.from(files).slice(0,100);let count=0;
 for(const file of input){
  if(file.size>5*1024*1024){toast(file.name+' 超过原型导入上限 5 MiB；可通过“状态演示”审阅大文件流程。');continue;}
  try{
   const bytes=new Uint8Array(await file.arrayBuffer());if(bytes.slice(0,8192).includes(0)){toast(file.name+' 包含零字节；原型仅导入 UTF-8 文本。');continue;}
   const content=new TextDecoder('utf-8',{fatal:true}).decode(bytes);
   const id='file-'+Date.now()+'-'+count,d=makeDocument(id,file.name,file.webkitRelativePath||file.name,content);
   d.bom=bytes[0]===239&&bytes[1]===187&&bytes[2]===191;state.docs.push(d);count++;
   if(count===1){if(folder)state.folderName=(file.webkitRelativePath||'文件夹').split('/')[0];openDocument(id,false);}
  }catch(e){toast(file.name+' 不是可解码的 UTF-8 文本；未替换无效字节。');}
 }
 renderSidebar();persist();if(count)toast('已载入 '+count+' 个 UTF-8 文件。浏览器导入不会创建本机文件监听。');
}
async function copyText(s){
 state.clipboard=s;try{await navigator.clipboard.writeText(s);toast('已复制。');}catch(e){toast('文字已放入原型剪贴板；浏览器未授予系统剪贴板权限。');}
}
function copyOrCut(cut){
 const ta=activeTextarea(),d=currentDoc();if(ta&&d){
  let a=ta.selectionStart,b=ta.selectionEnd;
  if(a===b&&state.settings['editor.copyEmptySelection']!==false){a=d.source.lastIndexOf('\n',Math.max(0,a-1))+1;const end=d.source.indexOf('\n',b);b=end<0?d.source.length:end+1;}
  copyText(d.source.slice(a,b));if(cut){ta.setSelectionRange(a,b);replaceSelection('',true);}return;
 }
 const s=window.getSelection();if(s&&!s.isCollapsed){copyText(s.toString());if(cut){document.execCommand('delete');const n=s.anchorNode?.parentElement?.closest('[data-block]');if(n)commitRichBlock(n,currentDoc());}}
}
async function pasteFromMenu(){
 let text=state.clipboard;try{text=await navigator.clipboard.readText();}catch(e){}
 if(!text){showDialog('粘贴文字','<p>浏览器未授予读取剪贴板权限。可直接在编辑区按 Ctrl+V，或在此输入文字。</p><textarea id="paste-text" style="width:100%;min-height:120px;border:1px solid var(--border);padding:10px;background:var(--editor);resize:vertical" aria-label="粘贴文字"></textarea>',[['取消',closeDialog],['插入',function(){const value=el('paste-text').value;closeDialog();replaceSelection(value,true);},'primary']]);return;}
 replaceSelection(text,true);
}
function selectAll(){const ta=activeTextarea();if(ta){ta.focus();ta.select();}else{const rich=document.querySelector('.editor-group[data-group="'+state.activeGroup+'"] .rich-document');if(rich){const r=document.createRange();r.selectNodeContents(rich);const s=getSelection();s.removeAllRanges();s.addRange(r);}}}
function clearMulti(){state.multi=null;const ta=activeTextarea();if(ta)updateSourceView(ta,currentDoc(),currentGroup());updateStatus();}
function goToLine(){
 const d=currentDoc();if(!d)return;forceSource('转到行在 Markdown 源码中定位。');
 showDialog('转到行','<label class="field">行号<input id="line-number" type="number" min="1" max="'+d.source.split('\n').length+'" value="1"></label>',[['取消',closeDialog],['转到',function(){const line=Math.max(1,Math.min(d.source.split('\n').length,Number(el('line-number').value)||1));closeDialog();const ta=activeTextarea(),offset=d.source.split('\n').slice(0,line-1).join('\n').length+(line>1?1:0);ta.focus();ta.setSelectionRange(offset,offset);ta.scrollTop=Math.max(0,(line-1)*24-80);syncSourceScroll(ta);updateStatus();},'primary']]);
}
function cycleTab(delta){const g=currentGroup();if(!g.tabs.length)return;activateDoc(g.id,g.tabs[(g.tabs.indexOf(g.active)+delta+g.tabs.length)%g.tabs.length]);}
function focusGroup(index){const g=state.groups[index];if(!g){toast('该编辑区域尚未打开。');return;}state.activeGroup=g.id;document.querySelectorAll('.editor-group').forEach(function(n){n.classList.toggle('focused',n.dataset.group===g.id);});activeTextarea()?.focus();updateStatus();}
function newWindow(){
 const d=currentDoc();persist();const url=new URL(window.location.href);url.searchParams.set('texziWindow',d?d.id:'todo');
 const opened=window.open(url.href,'texzi-review-'+Date.now(),'width=1150,height=800');
 if(opened)toast('已打开独立浏览器演示窗口；原生多窗口共享服务由 NSH 实现。');
 else showDialog('浏览器阻止了新窗口','<p>允许弹出窗口后可再次演示；原生 texzi 使用独立桌面窗口，此限制只属于浏览器原型。</p>',[['关闭',closeDialog,'primary']]);
}
function showEncodingDialog(){
 const d=currentDoc();showDialog('编码与换行','<p>当前原型实际导入、下载 UTF-8 文本，可保留 UTF-8 BOM 和选择 CRLF／LF。UTF-16、GB18030／GBK、Windows-1252 的解码、无损检测和“重新打开／按编码保存”属于原生实施验收项。</p><table><tr><th>当前文件</th><td>'+esc(d.name)+'</td></tr><tr><th>编码</th><td>'+d.encoding+(d.bom?'，带 BOM':'，无 BOM')+'</td></tr><tr><th>换行</th><td>'+d.eol+'</td></tr></table>',[['关闭',closeDialog,'primary']]);
}
function bannerAction(action,d){
 if(action==='compare'){
  showDialog('比较外部变更','<p>这是人工触发的冲突场景。请选择要保留的内容；原型不监听本机文件。</p><div class="demo-diff"><div><h3>当前编辑</h3><pre>'+esc(d.source)+'</pre></div><div><h3>磁盘版本（演示）</h3><pre>'+esc(d.externalSource||d.savedSource)+'</pre></div></div>',[['取消',closeDialog],['保留当前编辑',function(){d.banner=null;closeDialog();renderAll();}],['加载外部版本',function(){closeDialog();bannerAction('reload',d);},'primary']],true);return;
 }
 if(action==='reload'){
  showDialog('加载外部版本？','<p>当前未保存修改会被替换。原型将此操作记为一个可撤销步骤。</p>',[['取消',closeDialog],['加载外部版本',function(){editDocument(d,d.externalSource||d.savedSource,'external-reload',true);d.savedSource=d.source;d.banner=null;closeDialog();renderAll();},'primary']]);return;
 }
 if(action==='keep'){d.banner=null;toast('已保留编辑内容，仍为未保存状态。');}
 if(action==='accept-recovery'){d.banner=null;d.recovered=true;toast('已保留恢复内容；请保存或继续编辑。');}
 if(action==='discard-recovery'){showDialog('放弃恢复内容？','<p>将恢复为当前保存点。此演示操作可通过撤销恢复。</p>',[['取消',closeDialog],['放弃恢复',function(){editDocument(d,d.savedSource,'discard-recovery',true);d.banner=null;closeDialog();renderAll();},'danger']]);return;}
 if(action==='normal'){d.demoLarge=false;d.banner=null;d.readOnly=false;}
 if(action==='large-info'){showDialog('大文件策略','<p>当前是 100 MiB 大文件的界面状态演示，实际载入的是约 400 行小样本。它不能证明任何性能目标。</p><p>正式方案在达到 20 MiB 时启用渐进处理，不自动改成只读；达到 200 MiB 时先预估资源并提供选择。Markdown 排版准备可取消，用户可主动改用源码。完整测试方案见 PERFORMANCE_PLAN。</p>',[['关闭',closeDialog,'primary']]);return;}
 if(action==='unlock'){d.readOnly=false;d.banner=null;}
 if(action==='save-as'){saveCurrent(true);return;}
 if(action==='retry'){d.banner=null;toast('保存失败场景已解除；可再次下载文件。');}
 renderAll();schedulePersist();
}
function scenario(type){
 state.scene='main';state.find=null;state.multi=null;
 if(type==='large'){
  let d=docById('large-demo');if(!d){d=makeDocument('large-demo','server-large.log','日常文字/server-large.log',Array.from({length:400},function(_,i){return String(i+1).padStart(5,'0')+'  INFO  2026-11-15 12:00:00  文本日志样本；100 MiB 是场景标签，非实际载入大小。';}).join('\n'));state.docs.push(d);}
  d.demoLarge=true;d.banner='large';d.readOnly=false;state.groups=[makeGroup('g1',[d.id])];state.activeGroup='g1';
 }else if(type==='external'){
  const d=docById('todo')||state.docs[0];d.externalSource=d.savedSource+'\n外部程序追加：会议调整为 10:30。\n';editDocument(d,d.source+'\n本地编辑：补充验收事项。\n','scenario',true);d.banner='external';state.groups=[makeGroup('g1',[d.id])];state.activeGroup='g1';
 }else if(type==='recovery'){
  const id='recovery-'+Date.now(),d=makeDocument(id,'未命名-恢复','', '恢复的临时记录\n\n下午与团队确认：\n1. Markdown 所见即所得\n2. Win10 x64\n3. 中文输入法与异常恢复\n',true);d.banner='recovery';state.docs.push(d);state.groups=[makeGroup('g1',[id])];state.activeGroup='g1';
 }else if(type==='readonly'||type==='save-error'){
  const d=docById('todo')||state.docs[0];d.banner=type;d.readOnly=type==='readonly';state.groups=[makeGroup('g1',[d.id])];state.activeGroup='g1';
 }else if(type==='multi'){
  let d=docById('multi-demo');if(!d){d=makeDocument('multi-demo','批量编辑.json','日常文字/片段/批量编辑.json','{\n  "title": "alpha",\n  "label": "alpha",\n  "group": "alpha"\n}');state.docs.push(d);}
  state.groups=[makeGroup('g1',[d.id])];state.activeGroup='g1';renderAll();const ta=activeTextarea(),at=d.source.indexOf('alpha');ta.focus();ta.setSelectionRange(at,at+5);addNextSelection(true);return;
 }else if(type==='raw'){
  let d=docById('raw-demo');if(!d){d=makeDocument('raw-demo','语法保留.md','日常文字/工作笔记/语法保留.md','---\ntitle: texzi\ncustom: keep-me\n---\n\n# 原文保留演示\n\n普通段落可以直接编辑。\n\n::: custom-widget\n这个未知扩展块保留原始文本。\n:::\n\n'+TICK.repeat(3)+'mermaid\nflowchart LR\n  A --> B\n'+TICK.repeat(3)+'\n');state.docs.push(d);}
  state.groups=[makeGroup('g1',[d.id])];state.activeGroup='g1';
 }
 state.sidebar='files';state.sidebarVisible=true;renderAll();
}
function showScenarios(anchor){
 showMenu(anchor,[['模拟大文件模式','',function(){scenario('large');}],['模拟外部文件变更','',function(){scenario('external');}],['模拟异常恢复','',function(){scenario('recovery');}],['模拟只读文件','',function(){scenario('readonly');}],['模拟保存失败','',function(){scenario('save-error');}],null,['多光标批量编辑','可编辑',function(){scenario('multi');}],['未知 Markdown 语法保留','可编辑',function(){scenario('raw');}],['深色主题','',function(){updateSetting('appearance.theme','dark');}],['Zen 专注模式','',toggleZen]]);
}
function showPrototypeHelp(){
 showDialog('原型功能说明','<p>这是 texzi 的可运行交互原型，供审阅界面与行为。最终应用采用 NSH；此页面使用浏览器技术。</p><table><thead><tr><th>能力</th><th>当前原型</th></tr></thead><tbody><tr><td>文本输入、撤销、查找替换</td><td>真实可操作；历史采用浏览器样本实现，正则为 JavaScript RegExp（500 ms 预算），与正式 PCRE2 方案不同。</td></tr><tr><td>Markdown 所见即所得</td><td>标题、段落、列表、任务、基础表格、代码块可编辑；按块回写源码；切换模式不改文档。不是完整 Typora／NSH 富文本内核。</td></tr><tr><td>多光标</td><td>可选择多个相同内容并同时输入、替换、退格；不覆盖矩形选区、完整 Unicode 与输入法组合语义。</td></tr><tr><td>文件与文件夹</td><td>导入 UTF-8 文件（每个最多5 MiB、最多100个），保存为下载副本。不会监听或覆盖本机原文件。</td></tr><tr><td>标签、分屏、设置、Zen</td><td>可切换、关闭、固定、拖动标签，最多3个演示区域；常用设置实际生效，其他设置保存配置值并注明。</td></tr><tr><td>恢复与外部变更</td><td>浏览器会话恢复可用；系统崩溃、文件监听、原子保存与冲突来源由“状态演示”模拟。</td></tr><tr><td>新窗口与性能</td><td>新建浏览器演示窗口；不证明 NSH 原生多窗口、TSF、DPI或100 MiB性能。</td></tr></tbody></table><div class="review-callout">原型内容只保存在当前浏览器的本地存储，可通过“帮助 → 重置演示数据”清除。直接关闭页面不会写入你的本机原文件。</div>',[['开始审阅',closeDialog,'primary']],true);
}
function showReviewGuide(){
 showDialog('建议审阅路径','<ol style="line-height:2;font-size:13px"><li>主界面：修改文本，查找“文本”，展开替换，再撤销。</li><li>分屏编辑：修改任务项或表格，点击正文右上 &gt;_，核对源码，然后撤销。</li><li>拖动标签到另一编辑区域，调整分隔线；通过菜单打开新窗口。</li><li>设置：切换深色主题、字号、换行；搜索“恢复”。</li><li>状态演示：外部变更 → 比较；恢复 → 保留；大文件 → 查看策略。</li><li>查看 → Zen；两次 Esc 退出并恢复布局。</li></ol>',[['关闭',closeDialog,'primary']]);
}
function resetDemo(){
 showDialog('重置演示数据','<p>清除当前原型保存的文档、设置和布局，恢复初始样本。此操作不影响你导入文件的原件。</p>',[['取消',closeDialog],['重置',function(){localStorage.removeItem(STORAGE_KEY);state.docs=initialDocuments();state.settings=Object.assign({},defaultSettings);closeDialog();switchScene('main');toast('已恢复初始演示数据。');},'danger']]);
}
function paletteEntries(commandsOnly){
 const commands=[
  ['新建文本','Ctrl+N',newDocument],['打开文件','Ctrl+O',function(){el('file-input').click();}],['打开文件夹','Ctrl+K Ctrl+O',function(){el('folder-input').click();}],
  ['Markdown：切换源码 / 所见即所得','>_',function(){toggleMarkdown();}],['拆分编辑区域','Ctrl+\\',splitCurrent],['Zen 模式','Ctrl+K Z',toggleZen],
  ['设置','Ctrl+,',function(){switchScene('settings');}],['查找','Ctrl+F',function(){openFind(false);}],['替换','Ctrl+H',function(){openFind(true);}],
  ['下载当前文件','Ctrl+S',function(){saveCurrent(false);}],['新建窗口','Ctrl+Shift+N',newWindow],['显示 / 隐藏侧栏','Ctrl+K Ctrl+B',toggleSidebar],
  ['主题：浅色','',function(){updateSetting('appearance.theme','light');}],['主题：深色','',function(){updateSetting('appearance.theme','dark');}],
  ['状态演示：外部变更','',function(){scenario('external');}],['状态演示：异常恢复','',function(){scenario('recovery');}],['状态演示：大文件','',function(){scenario('large');}],
  ['多光标批量编辑','',function(){scenario('multi');}],['原型说明','',showPrototypeHelp]
 ].map(function(c){return{name:c[0],detail:c[1],run:c[2],kind:'command'};});
 const docs=state.docs.map(function(d){return{name:d.name,detail:d.path||'临时文本',run:function(){openDocument(d.id,false);},kind:'file'};});
 return commandsOnly?commands:docs.concat(commands);
}
function openPalette(commandsOnly){
 closeMenu();let entries=paletteEntries(commandsOnly),filtered=entries,selected=0;
 el('dialog-root').innerHTML='<div class="backdrop palette-backdrop"><section class="dialog command-palette" role="dialog" aria-modal="true" aria-label="搜索文件或命令"><input id="palette-query" placeholder="'+(commandsOnly?'输入命令名称':'搜索文件，或输入 > 查找命令')+'" aria-label="命令查询"><div class="palette-results"></div><div class="palette-hint">↑ ↓ 选择　Enter 打开　Esc 关闭</div></section></div>';
 function paint(){const box=el('dialog-root').querySelector('.palette-results');box.innerHTML=filtered.slice(0,30).map(function(e,i){return'<button class="palette-item '+(i===selected?'selected':'')+'" data-palette="'+i+'">'+icon(e.kind==='file'?'file':'chevron')+'<strong>'+esc(e.name)+'</strong><span>'+esc(e.detail)+'</span></button>';}).join('')||'<div class="settings-empty" style="padding:25px">没有匹配项</div>';box.querySelectorAll('[data-palette]').forEach(function(b){b.onclick=function(){const entry=filtered[Number(b.dataset.palette)];closeDialog();entry.run();};});}
 paint();const input=el('palette-query');input.focus();
 input.oninput=function(){const q=input.value.trim().toLowerCase(),only=q.startsWith('>'),needle=only?q.slice(1).trim():q;filtered=entries.filter(function(e){return(!only||e.kind==='command')&&(e.name+' '+e.detail).toLowerCase().includes(needle);});selected=0;paint();};
 input.onkeydown=function(e){if(e.key==='ArrowDown'){e.preventDefault();selected=Math.min(selected+1,Math.min(29,filtered.length-1));paint();}if(e.key==='ArrowUp'){e.preventDefault();selected=Math.max(0,selected-1);paint();}if(e.key==='Enter'&&filtered[selected]){e.preventDefault();const entry=filtered[selected];closeDialog();entry.run();}if(e.key==='Escape')closeDialog();};
}
function perform(action){if(action==='new')newDocument();if(action==='open')el('file-input').click();if(action==='folder')el('folder-input').click();}
let chordUntil=0,lastEscape=0;
function handleKeyboard(e){
 if(e.isComposing||state.composing||e.keyCode===229)return;
 const key=e.key.toLowerCase(),ctrl=e.ctrlKey||e.metaKey;
 if(key==='escape'){
  if(el('dialog-root').firstChild){closeDialog();e.preventDefault();return;}
  if(el('menu-root').firstChild){closeMenu();e.preventDefault();return;}
  if(state.find){closeFind();e.preventDefault();return;}
  if(state.multi){clearMulti();e.preventDefault();return;}
  if(state.zen&&Date.now()-lastEscape<800){toggleZen();e.preventDefault();}
  lastEscape=Date.now();return;
 }
 const textField=e.target.matches('input,select')||e.target.closest('.dialog,.find-widget');
 if(chordUntil>Date.now()){
  chordUntil=0;if(key==='z'){e.preventDefault();toggleZen();return;}if(ctrl&&key==='b'){e.preventDefault();toggleSidebar();return;}if(ctrl&&key==='o'){e.preventDefault();el('folder-input').click();return;}
 }
 if(!ctrl){if(e.shiftKey&&e.altKey&&e.key==='ArrowDown'&&!textField){e.preventDefault();duplicateLine();}return;}
 if(textField&&['a','c','x','v','z','y'].includes(key))return;
 let action=null;
 if(key==='k'&&!e.shiftKey){action=function(){chordUntil=Date.now()+1500;toast('等待组合键：Z / Ctrl+B / Ctrl+O');};}
 else if(key==='p')action=function(){openPalette(e.shiftKey);};
 else if(key==='f')action=e.shiftKey?function(){state.sidebar='search';state.sidebarVisible=true;state.scene=state.groups.length>1?'split':'main';renderAll();el('workspace-query')?.focus();}:function(){openFind(false);};
 else if(key==='h')action=function(){openFind(true);};
 else if(key==='n')action=e.shiftKey?newWindow:newDocument;
 else if(key==='o')action=function(){el('file-input').click();};
 else if(key==='s')action=function(){saveCurrent(e.shiftKey);};
 else if(key==='z'&&!textField)action=function(){historyAction(e.shiftKey);};
 else if(key==='y'&&!textField)action=function(){historyAction(true);};
 else if(key===',')action=function(){switchScene('settings');};
 else if(key==='\\')action=splitCurrent;
 else if(key==='b'&&!textField)action=function(){if(currentDoc()&&groupMode(currentGroup(),currentDoc())==='wysiwyg'&&e.target.closest('.rich-document'))formatRich('bold');else toggleSidebar();};
 else if(key==='i'&&!textField)action=function(){formatRich('italic');};
 else if(key==='d'&&!textField)action=function(){addNextSelection(false);};
 else if(key==='l'&&e.shiftKey&&!textField)action=function(){addNextSelection(true);};
 else if(key==='g')action=goToLine;
 else if(key==='w')action=function(){closeTab(state.activeGroup,currentGroup().active);};
 else if(key==='tab')action=function(){cycleTab(e.shiftKey?-1:1);};
 else if(/^[123]$/.test(key))action=function(){focusGroup(Number(key)-1);};
 if(action){e.preventDefault();action();}
}
function boot(){
 document.querySelector('.app-icon').innerHTML=icon('file');
 document.querySelector('.search-icon').innerHTML=icon('search');
 el('activity-files').innerHTML=icon('file');el('activity-search').innerHTML=icon('search');el('activity-settings').innerHTML=icon('settings');
 buildMenus();
 document.querySelectorAll('[data-scene]').forEach(function(b){b.onclick=function(){switchScene(b.dataset.scene);};});
 el('activity-files').onclick=function(){if(state.scene!=='settings'&&state.sidebar==='files')state.sidebarVisible=!state.sidebarVisible;else state.sidebarVisible=true;state.scene=state.groups.length>1?'split':'main';state.sidebar='files';renderAll();};
 el('activity-search').onclick=function(){state.scene=state.groups.length>1?'split':'main';state.sidebar='search';state.sidebarVisible=true;renderAll();el('workspace-query')?.focus();};
 el('activity-settings').onclick=function(){switchScene('settings');};
 el('quick-open').onclick=function(){openPalette(false);};el('prototype-help').onclick=showPrototypeHelp;
 el('scenario-trigger').onclick=function(e){showScenarios(e.currentTarget);};
 el('exit-zen').onclick=toggleZen;
 el('file-input').onchange=function(e){loadFiles(e.target.files,false);e.target.value='';};
 el('folder-input').onchange=function(e){loadFiles(e.target.files,true);e.target.value='';};
 el('minimize').onclick=function(){el('app').style.display='none';el('minimized-state').hidden=false;};
 el('restore-window').onclick=function(){el('app').style.display='flex';el('minimized-state').hidden=true;};
 el('maximize').onclick=function(){document.body.classList.toggle('maximized');};
 el('close-window').onclick=function(){showDialog('结束本次演示','<p>关闭此对话框可继续审阅；浏览器页面可以直接关闭。原型已按当前设置保存浏览器会话。</p>',[['继续审阅',closeDialog,'primary']]);};
 el('sidebar-resizer').onpointerdown=function(e){const h=e.currentTarget,start=e.clientX,width=el('sidebar').offsetWidth;h.setPointerCapture(e.pointerId);h.onpointermove=function(ev){document.documentElement.style.setProperty('--sidebar-width',Math.max(180,Math.min(420,width+ev.clientX-start))+'px');};h.onpointerup=function(){h.onpointermove=null;};};
 document.addEventListener('pointerdown',function(e){if(!e.target.closest('.menu,.menubar-button,#scenario-trigger,.group-more,.tab,#status-right'))closeMenu();});
 document.addEventListener('keydown',handleKeyboard);
 window.addEventListener('beforeunload',persist);
 const windowDoc=new URL(location.href).searchParams.get('texziWindow');
 if(windowDoc&&docById(windowDoc)){state.groups=[makeGroup('g1',[windowDoc])];state.activeGroup='g1';state.scene='main';}
 renderAll();
 if(restoredSession)setTimeout(function(){toast('已恢复此浏览器保存的演示会话。帮助菜单可重置样本。');},250);
}
window.texziReview={
 getState:function(){return state;},searchCore,parseMarkdown,serializeBlock,expandReplacement,
 switchScene,scenario,toggleMarkdown,historyAction,editDocument,openFind,runFind,
 reset:function(){state.docs=initialDocuments();state.settings=Object.assign({},defaultSettings);state.find=null;state.multi=null;switchScene('main');},
 capabilities:{native:false,storage:'browser-localStorage',regex:'JavaScript RegExp in Worker',fileEncoding:'UTF-8',maxImportMiB:5,maxGroups:3}
};
boot();
