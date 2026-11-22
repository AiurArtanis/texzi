#!/usr/bin/env node
// Build a self-contained handbook. Only Node and the vendored Marked are needed.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {marked} from './vendor/marked.mjs';

const ROOT=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const read=(p)=>fs.readFileSync(path.join(ROOT,p),'utf8');
const image=(p)=>'data:image/png;base64,'+fs.readFileSync(path.join(ROOT,p)).toString('base64');
const commands=JSON.parse(read('data/commands.json'));
const settings=JSON.parse(read('data/settings.json'));
const entries=[
 {id:'overview',title:'交付总览',group:'从这里开始'},
 {id:'concepts-page',title:'确认概念图'},
 {id:'prototype-page',title:'交互原型'},
 {id:'prd',title:'完整 PRD',file:'docs/PRD.md',group:'产品与体验'},
 {id:'ui-spec',title:'功能与界面规格',file:'docs/UI_SPEC.md'},
 {id:'commands',title:'命令与默认快捷键',file:'docs/COMMANDS.md'},
 {id:'settings',title:'完整设置表',file:'docs/SETTINGS.md'},
 {id:'technical',title:'技术方案',file:'docs/TECHNICAL_DESIGN.md',group:'实施与验证'},
 {id:'performance',title:'性能测试计划',file:'docs/PERFORMANCE_PLAN.md'},
 {id:'acceptance',title:'112 条产品验收',file:'docs/ACCEPTANCE.md'},
 {id:'handoff',title:'Agent 团队交接',file:'docs/HANDOFF.md'},
 {id:'decisions',title:'决策、假设与未决项',file:'docs/DECISIONS.md'},
 {id:'prototype-guide',title:'原型运行与边界',file:'docs/PROTOTYPE_GUIDE.md',group:'使用与证据'},
 {id:'verification',title:'本次验证报告',file:'docs/VERIFICATION.md'},
 {id:'reference-research',title:'quiet.md 实际能力核实',file:'docs/REFERENCE_RESEARCH.md'},
 {id:'sources',title:'来源与核实',file:'docs/SOURCES.md'},
 {id:'readme',title:'文件与运行索引',file:'README.md'}
];
const escapeHTML=(s)=>String(s).replace(/[&<>"']/g,(c)=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const docMap=new Map(entries.filter(x=>x.file).map(x=>[path.basename(x.file),x.id]));
const flow=read('review/document-flow.svg');
const sources=new Map();
for(const s of commands.baseline.sources||[])sources.set(s.url,{url:s.url,title:s.title,kind:s.kind,verification:s.read?'source_reviewed':'reference',documents:['docs/COMMANDS.md']});
let articles='';
for(const entry of entries.filter(x=>x.file)){
 const markdown=read(entry.file);
 for(const m of markdown.matchAll(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g)){
  const s=sources.get(m[2])||{url:m[2],title:m[1],kind:'document_reference',verification:'referenced_in_spec',documents:[]};
  if(!s.documents.includes(entry.file))s.documents.push(entry.file);sources.set(m[2],s);
 }
 for(const m of markdown.matchAll(/^\[([^\]]+)\]:\s+(https?:\/\/\S+)(?:\s+"([^"]+)")?/gm)){
  const s=sources.get(m[2])||{url:m[2],title:m[3]||m[1],kind:'document_reference',verification:'referenced_in_spec',documents:[]};
  if(!s.documents.includes(entry.file))s.documents.push(entry.file);sources.set(m[2],s);
 }
 let html=marked.parse(markdown,{gfm:true,breaks:false});
 html=html.replace(/href="([^"]+)"/g,(all,href)=>{
  if(/^https?:/.test(href))return'href="'+href+'" target="_blank" rel="noreferrer"';
  const base=path.basename(href.split('#')[0]);
  if(docMap.has(base))return'href="#'+docMap.get(base)+'"';
  if(href.includes('concepts/'))return'href="#concepts-page"';
  if(href.includes('prototype/'))return'href="#prototype-page"';
  return all;
 });
 html=html.replace(/<pre><code class="language-mermaid">([\s\S]*?)<\/code><\/pre>/g,(_,code)=>'<figure class="diagram">'+flow+'</figure><details><summary>查看 Mermaid 源图</summary><pre><code>'+code+'</code></pre></details>');
 html=html.replace(/<table>/g,'<div class="table-wrap"><table>').replace(/<\/table>/g,'</table></div>');
 articles+='<article class="page document" id="'+entry.id+'"><div class="doc-meta">'+escapeHTML(entry.file)+' · v1.0</div>'+html+'</article>\n';
}
fs.writeFileSync(path.join(ROOT,'data/source-index.json'),JSON.stringify({schemaVersion:1,verifiedAt:'2026-11-15',note:'Document references are not automatically runtime-verified. Fixed source reviews and native tests have different statuses.',sources:[...sources.values()]},null,2)+'\n');
const nav=entries.map((e,i)=>(e.group?'<div class="nav-label">'+e.group+'</div>':'')+'<a href="#'+e.id+'"><span>'+String(i+1).padStart(2,'0')+'</span>'+e.title+'</a>').join('\n');
const prototype=read('prototype/index.html');
const data='window.HANDBOOK='+JSON.stringify({entries:entries.map(({id,title})=>({id,title})),prototype}).replaceAll('</','<\\/')+';';
const frCount=new Set((read('docs/PRD.md').match(/FR-[A-Z]+-\d+/g)||[])).size;
const acCount=new Set((read('docs/ACCEPTANCE.md').match(/AC-\d{3}/g)||[])).size;
const replacements={CSS:read('review/handbook.css'),NAV:nav,ARTICLES:articles,DATA:data,MAIN_IMAGE:image('concepts/01-main.png'),SPLIT_IMAGE:image('concepts/02-markdown-wysiwyg-split.png'),SETTINGS_IMAGE:image('concepts/03-settings.png'),FR_COUNT:frCount,AC_COUNT:acCount,COMMAND_COUNT:commands.commands.length,SETTING_COUNT:settings.settings.length};
const output=read('review/handbook-shell.html').replace(/\{\{([A-Z_]+)\}\}/g,(_,key)=>replacements[key]??'');
const target=path.join(ROOT,'texzi-design-handbook.html');
fs.writeFileSync(target,output);
console.log('Built handbook:',fs.statSync(target).size,'bytes;',entries.length,'sections;',sources.size,'source URLs');
