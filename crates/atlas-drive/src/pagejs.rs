// SPDX-License-Identifier: GPL-2.0-or-later
//! Scripts that run inside the page (Runtime.evaluate, returnByValue). Each is one expression that
//! evaluates to JSON-able data and never writes to the page except the cursor overlay and the
//! WebMCP recorder (armed on request). `{ARGS}` is replaced by the JSON arguments.

pub fn with_args(script: &str, args: &serde_json::Value) -> String {
    script.replace("{ARGS}", &args.to_string())
}

/// Active element, one short string.
pub const FOCUS: &str = r#"(()=>{const e=document.activeElement;if(!e||e===document.body||e===document.documentElement)return '';
const n=(e.getAttribute('aria-label')||e.labels&&e.labels[0]&&e.labels[0].innerText||e.placeholder||e.innerText||e.value||'').trim().slice(0,40);
return e.tagName.toLowerCase()+(e.type&&e.tagName==='INPUT'?'['+e.type+']':'')+(e.id?'#'+e.id:'')+(n?' "'+n+'"':'')})()"#;

/// Wait (inside the page, MutationObserver, no polling) until a selector or text exists / is gone.
/// args: {kind:'selector'|'text'|'gone-selector'|'gone-text'|'stable', value, ms}
pub const WAIT: &str = r#"((a)=>new Promise((res)=>{
const test=()=>{
 if(a.kind==='selector')return !!document.querySelector(a.value);
 if(a.kind==='gone-selector')return !document.querySelector(a.value);
 const t=document.body?document.body.innerText:'';
 if(a.kind==='text')return t.indexOf(a.value)>=0;
 if(a.kind==='gone-text')return t.indexOf(a.value)<0;
 return false;};
if(a.kind==='stable'){
 let last=performance.now(),raf=0;const q=a.quiet||120;
 const mo=new MutationObserver(()=>{last=performance.now()});
 mo.observe(document,{subtree:true,childList:true,attributes:true,characterData:true});
 let ro;try{ro=new ResizeObserver(()=>{last=performance.now()});ro.observe(document.documentElement);}catch(e){}
 const tick=()=>{ if(performance.now()-last>=q){mo.disconnect();ro&&ro.disconnect();res(true);} else raf=requestAnimationFrame(tick);};
 requestAnimationFrame(()=>requestAnimationFrame(tick));return;}
if(test()){res(true);return;}
const mo=new MutationObserver(()=>{if(test()){mo.disconnect();res(true);}});
mo.observe(document,{subtree:true,childList:true,attributes:true,characterData:true});
}))({ARGS})"#;

/// Links: kind, text, same-origin flag, deduped.
pub const LINKS: &str = r#"((a)=>{
const out=[],seen=new Map();const here=location.origin;
const kindOf=(el,u)=>{
 if(u.protocol==='mailto:'||u.protocol==='tel:')return u.protocol.slice(0,-1);
 if(u.hash&&u.href.split('#')[0]===location.href.split('#')[0])return 'anchor';
 if(el.hasAttribute('download'))return 'download';
 if(el.closest('nav,[role=navigation]'))return 'nav';
 if(el.closest('header,[role=banner]'))return 'header';
 if(el.closest('footer,[role=contentinfo]'))return 'footer';
 if(el.closest('aside,[role=complementary]'))return 'aside';
 if(el.closest('main,article,[role=main]'))return 'content';
 return 'other';};
for(const el of document.querySelectorAll('a[href]')){
 let u;try{u=new URL(el.href,location.href)}catch(e){continue}
 if(u.protocol==='javascript:')continue;
 const t=(el.getAttribute('aria-label')||el.innerText||el.title||(el.querySelector('img')&&el.querySelector('img').alt)||'').replace(/\s+/g,' ').trim().slice(0,120);
 const k=kindOf(el,u);
 if(a.kind&&a.kind!==k)continue;
 const same=u.origin===here;
 if(a.same_origin===true&&!same)continue;
 if(a.same_origin===false&&same)continue;
 const key=u.href+'\u0000'+t;
 if(seen.has(key)){seen.get(key).n++;continue}
 const r={u:u.href,t:t,k:k,s:same?1:0,n:1,b:el.target==='_blank'?1:0,r:(el.rel||'').slice(0,40)};
 seen.set(key,r);out.push(r);
 if(out.length>=(a.limit||500))break;}
return {origin:here,items:out};})({ARGS})"#;

/// Forms: schema of every form and of loose controls.
pub const FORMS: &str = r#"((a)=>{
const lab=(el)=>{
 if(el.labels&&el.labels.length)return el.labels[0].innerText.replace(/\s+/g,' ').trim().slice(0,80);
 const al=el.getAttribute('aria-label');if(al)return al.slice(0,80);
 const by=el.getAttribute('aria-labelledby');if(by){const t=by.split(' ').map(i=>{const x=document.getElementById(i);return x?x.innerText:''}).join(' ').trim();if(t)return t.slice(0,80);}
 if(el.tagName==='BUTTON'||el.type==='submit'||el.type==='button')return (el.innerText||el.value||'').replace(/\s+/g,' ').trim().slice(0,80);
 return (el.placeholder||el.title||'').slice(0,80);};
const field=(el)=>{
 const tag=el.tagName.toLowerCase();const type=tag==='input'?(el.type||'text'):tag;
 const f={name:el.name||el.id||'',type:type,label:lab(el)};
 if(el.required)f.required=1;if(el.disabled)f.disabled=1;if(el.readOnly)f.readonly=1;
 if(el.pattern)f.pattern=el.pattern;if(el.maxLength>0&&el.maxLength<100000)f.max_length=el.maxLength;
 if(el.min)f.min=el.min;if(el.max)f.max=el.max;if(el.step)f.step=el.step;
 if(el.autocomplete&&el.autocomplete!=='on'&&el.autocomplete!=='off')f.autocomplete=el.autocomplete;
 if(type==='password')f.value=el.value?'(set)':'';
 else if(type==='checkbox'||type==='radio'){f.checked=el.checked?1:0;f.value=el.value;}
 else if(tag==='select'){f.options=[...el.options].slice(0,50).map(o=>({v:o.value,t:o.text.trim().slice(0,60),s:o.selected?1:0}));f.multiple=el.multiple?1:0;}
 else if(el.value)f.value=String(el.value).slice(0,80);
 if(el.form&&el.form.id)f.form=el.form.id;
 return f;};
const sel='input:not([type=hidden]),select,textarea,button[type=submit],input[type=submit]';
const forms=[...document.forms].slice(0,30).map((fm,i)=>({
 id:fm.id||fm.name||('form'+i),action:fm.action,method:(fm.method||'get').toLowerCase(),
 fields:[...fm.querySelectorAll(sel)].slice(0,80).map(field)}));
const loose=[...document.querySelectorAll(sel)].filter(e=>!e.form).slice(0,60).map(field);
return {forms:forms,loose:loose};})({ARGS})"#;

/// Trap scan: text a person cannot see (or is not meant to), and instruction-like text.
pub const TRAPS: &str = r#"((a)=>{
const INJ=/(ignore|disregard|forget|override)\b[^.]{0,40}\b(previous|prior|above|earlier|all|any)\b[^.]{0,30}\b(instruction|prompt|rule|message)s?|system prompt|you are (now |actually )?(an? |the )?(ai|assistant|model|llm|agent|chatbot)|as an ai\b|(do not|don't|never) (tell|reveal|mention|inform|show)\b[^.]{0,30}\b(user|human|person)|<\|im_(start|end)\|>|\[\/?INST\]|###\s*(system|instruction)|(send|post|forward|exfiltrate|email|upload)\b[^.]{0,60}\b(password|token|cookie|secret|credential|api[ _-]?key|conversation|chat history|private)|new instructions?:|important:?\s*(ai|assistant|agent|llm)|(ai|llm|assistant|agent)s?,?\s+(must|should|please|need to)\b/i;
const out=[];const cap=a.limit||60;
const path=(el)=>{let s=el.tagName.toLowerCase();if(el.id)s+='#'+el.id;else if(el.classList&&el.classList.length)s+='.'+[...el.classList].slice(0,2).join('.');return s;};
const add=(kind,el,text,extra)=>{
 text=(text||'').replace(/\s+/g,' ').trim();if(!text)return;
 if(out.length>=cap)return;
 const inj=INJ.test(text);
 out.push(Object.assign({kind:kind,at:path(el),text:text.slice(0,200),len:text.length,injection:inj?1:0},extra||{}));};
const rgb=(c)=>{const m=c.match(/[\d.]+/g);return m?m.map(Number):null;};
const bgOf=(el)=>{for(let e=el;e;e=e.parentElement){const c=rgb(getComputedStyle(e).backgroundColor);if(c&&(c.length<4||c[3]>0.1))return c;}return [255,255,255,1];};
const lum=(c)=>{const f=(v)=>{v/=255;return v<=0.03928?v/12.92:Math.pow((v+0.055)/1.055,2.4)};return 0.2126*f(c[0])+0.7152*f(c[1])+0.0722*f(c[2]);};
const vw=innerWidth,vh=innerHeight;
let n=0;const all=document.body?document.body.querySelectorAll('*'):[];
for(const el of all){
 if(++n>(a.max_nodes||8000))break;
 const tag=el.tagName;if(tag==='SCRIPT'||tag==='STYLE'||tag==='NOSCRIPT'||tag==='TEMPLATE'||tag==='OPTION'||tag==='OPTGROUP'||tag==='DATALIST'||tag==='TITLE'||tag==='HEAD')continue;
 let own='';for(const c of el.childNodes)if(c.nodeType===3)own+=c.nodeValue;
 own=own.replace(/\s+/g,' ').trim();
 let cs=null;
 if(own.length>0){
  cs=getComputedStyle(el);
  const r=el.getBoundingClientRect();
  if(cs.display==='none'||cs.visibility==='hidden'||cs.visibility==='collapse'){add('display-none',el,own);}
  else if(parseFloat(cs.opacity)<0.05){add('opacity-0',el,own);}
  else if(parseFloat(cs.fontSize)<2){add('tiny-font',el,own);}
  else if(r.width<2||r.height<2){if(cs.overflow!=='visible'||cs.clip!=='auto'||r.width<2&&r.height<2)add('zero-size',el,own);}
  else if(r.right<-100||r.bottom<-100||r.left>vw+4000||r.top>vh+20000||parseInt(cs.textIndent)<-500){add('offscreen',el,own);}
  else {
   const fg=rgb(cs.color),bg=bgOf(el);
   if(fg){const l1=lum(fg),l2=lum(bg);const ratio=(Math.max(l1,l2)+0.05)/(Math.min(l1,l2)+0.05);
    if(ratio<1.15&&(fg.length<4||fg[3]>0.1))add('low-contrast',el,own,{ratio:+ratio.toFixed(2)});}
   if(el.closest('[aria-hidden=true]')&&!el.matches('svg,svg *,i,[role=presentation]'))add('aria-hidden',el,own);
  }
 }
 for(const at of ['alt','title','aria-label','placeholder','aria-description']){
  const v=el.getAttribute(at);
  if(v&&v.length>30&&INJ.test(v))add('attr:'+at,el,v);}
 for(const at of el.attributes){
  if(at.name.startsWith('data-')&&at.value.length>40&&/\s/.test(at.value)&&INJ.test(at.value))add('attr:'+at.name,el,at.value);}
 if(a.deep){
  if(!cs)cs=getComputedStyle(el);
  if(cs.display!=='none'){
   for(const ps of ['::before','::after']){
    const c=getComputedStyle(el,ps).content;
    if(c&&c.length>34&&c[0]==='"'&&/\s/.test(c)&&INJ.test(c))add('css-content',el,c.slice(1,-1));}}}
}
// comments and metas
const tw=document.createTreeWalker(document.documentElement,NodeFilter.SHOW_COMMENT);let cn;let nc=0;
while((cn=tw.nextNode())&&nc++<2000){const t=cn.nodeValue.trim();if(t.length>20&&(INJ.test(t)||a.all_comments))add('html-comment',cn.parentElement||document.documentElement,t);}
for(const m of document.querySelectorAll('meta[name],meta[property]')){
 const v=m.content||'';if(v.length>30&&INJ.test(v))add('meta:'+(m.getAttribute('name')||m.getAttribute('property')),m,v);}
for(const f of document.querySelectorAll('iframe')){const r=f.getBoundingClientRect();if((r.width<2||r.height<2)&&f.src)add('hidden-iframe',f,f.src);}
for(const t of document.querySelectorAll('noscript')){const v=t.textContent||'';if(v.trim().length>30&&INJ.test(v))add('noscript',t,v);}
return {found:out,scanned:n};})({ARGS})"#;

/// A cheap fingerprint of the document: the element count and the length of its text. A cached trap scan is
/// reused while it is unchanged (a page that swaps text without changing either is rescanned by `traps`).
pub const PROBE: &str =
    "document.getElementsByTagName('*').length+':'+document.documentElement.textContent.length";

/// Main content, tables, metadata, structured data. Excludes what the trap scan would flag.
pub const EXTRACT: &str = r#"((a)=>{
const vis=(el)=>{const cs=getComputedStyle(el);if(cs.display==='none'||cs.visibility==='hidden'||parseFloat(cs.opacity)<0.05)return false;
 const r=el.getBoundingClientRect();return !(r.width<2&&r.height<2);};
const clean=(t)=>t.replace(/[ \t ]+/g,' ').replace(/\n\s*\n+/g,'\n').trim();
const SKIP='script,style,noscript,template,nav,aside,footer,header,form,svg,iframe,[aria-hidden=true],[hidden],[role=navigation],[role=banner],[role=contentinfo],[role=complementary],[role=search]';
const textOf=(root)=>{const lines=[];
 const walk=(el)=>{
  if(el.matches&&el.matches(SKIP)&&el!==root)return;
  if(el.nodeType===1&&!vis(el))return;
  let buf='';
  for(const c of el.childNodes){
   if(c.nodeType===3)buf+=c.nodeValue;
   else if(c.nodeType===1){
    const t=c.tagName;
    if(/^(P|DIV|LI|H[1-6]|BLOCKQUOTE|PRE|TR|SECTION|ARTICLE|UL|OL|DL|DT|DD|FIGCAPTION|TABLE)$/.test(t)){
     if(buf.trim()){lines.push(buf);buf='';}
     if(/^H[1-6]$/.test(t)){const x=c.innerText.trim();if(x&&vis(c))lines.push('#'.repeat(+t[1])+' '+x);}
     else walk(c);
    } else if(t==='BR'){lines.push(buf);buf='';}
    else if(c.matches(SKIP)){}
    else if(vis(c)){ if(t==='A'||t==='SPAN'||t==='B'||t==='I'||t==='EM'||t==='STRONG'||t==='CODE'||t==='SMALL'||t==='MARK'||t==='SUB'||t==='SUP'||t==='TIME'||t==='ABBR')buf+=(c.matches('[aria-hidden=true]')?'':c.innerText||c.textContent);else{ if(buf.trim()){lines.push(buf);buf='';} walk(c);} }
   }}
  if(buf.trim())lines.push(buf);};
 walk(root);return clean(lines.map(l=>l.replace(/\s+/g,' ').trim()).filter(Boolean).join('\n'));};
let main=document.querySelector('main,[role=main],article')||null;
if(!main){let best=null,bs=0;for(const el of document.body.querySelectorAll('div,section')){
  if(el.matches(SKIP))continue;const ps=el.querySelectorAll(':scope > p, :scope > div > p').length;const s=ps*50+Math.min(2000,(el.innerText||'').length)/20;if(ps>=2&&s>bs){bs=s;best=el;}}
 main=best||document.body;}
const maxc=a.max_chars||8000;
let text=textOf(main);let cut=false;if(text.length>maxc){text=text.slice(0,maxc);cut=true;}
const meta={};
const mt=(k,v)=>{if(v&&!meta[k])meta[k]=v.slice(0,300)};
mt('title',document.title);mt('lang',document.documentElement.lang);
const q=(s)=>document.querySelector(s);
mt('description',(q('meta[name=description]')||{}).content);
mt('canonical',(q('link[rel=canonical]')||{}).href);
mt('author',(q('meta[name=author]')||{}).content);
mt('published',(q('meta[property="article:published_time"]')||q('time[datetime]')||{}).content||(q('time[datetime]')||{}).dateTime);
for(const m of document.querySelectorAll('meta[property^="og:"],meta[name^="twitter:"]'))mt(m.getAttribute('property')||m.getAttribute('name'),m.content);
const tables=[];
for(const t of [...main.querySelectorAll('table')].slice(0,a.max_tables||5)){
 if(!vis(t))continue;const rows=[...t.rows].slice(0,a.max_rows||40).map(r=>[...r.cells].slice(0,20).map(c=>c.innerText.replace(/\s+/g,' ').trim().slice(0,80)));
 const cap=t.caption?t.caption.innerText.trim():'';tables.push({caption:cap,rows:rows,total_rows:t.rows.length});}
const ld=[];
for(const s of document.querySelectorAll('script[type="application/ld+json"]')){try{const j=JSON.parse(s.textContent);ld.push(j);}catch(e){}
 if(ld.length>=5)break;}
const micro=[];
for(const el of [...document.querySelectorAll('[itemscope][itemtype]')].slice(0,10)){
 const o={type:el.getAttribute('itemtype')};for(const p of el.querySelectorAll('[itemprop]')){if(p.closest('[itemscope]')!==el)continue;o[p.getAttribute('itemprop')]=(p.content||p.getAttribute('datetime')||p.getAttribute('href')||p.innerText||'').trim().slice(0,200);}micro.push(o);}
const heads=[...main.querySelectorAll('h1,h2,h3')].filter(vis).slice(0,40).map(h=>h.tagName.toLowerCase()+' '+h.innerText.replace(/\s+/g,' ').trim().slice(0,100));
return {meta:meta,headings:heads,text:text,truncated:cut,tables:tables,ld_json:ld,microdata:micro,words:text.split(/\s+/).filter(Boolean).length};})({ARGS})"#;

/// Compact DOM outline: tag#id.class (children count, text length), depth-limited.
pub const OUTLINE: &str = r#"((a)=>{
const maxd=a.depth||4,cap=a.max_nodes||120;let n=0;const out=[];
const walk=(el,d)=>{
 if(n>=cap)return;n++;
 const t=el.tagName.toLowerCase();let s=t+(el.id?'#'+el.id:'')+(el.classList&&el.classList.length?'.'+[...el.classList].slice(0,3).join('.'):'');
 const role=el.getAttribute('role');if(role)s+='[role='+role+']';
 const kids=el.children.length;let own=0;for(const c of el.childNodes)if(c.nodeType===3)own+=c.nodeValue.trim().length;
 out.push(' '.repeat(d)+s+(kids?' ('+kids+')':'')+(own?' "'+own+'ch"':''));
 if(d<maxd)for(const c of el.children){if(/^(SCRIPT|STYLE|LINK|META|NOSCRIPT|TEMPLATE|SVG|PATH)$/i.test(c.tagName))continue;walk(c,d+1);}
 else if(kids)out.push(' '.repeat(d+1)+'… '+kids+' children');};
const root=a.selector?document.querySelector(a.selector):document.body;
if(!root)return {error:'no match for '+a.selector};
walk(root,0);return {lines:out,truncated:n>=cap};})({ARGS})"#;

/// Framework and platform detection.
pub const FRAMEWORKS: &str = r#"(()=>{
const f=[];const add=(n,v)=>f.push(v?n+' '+v:n);const w=window;
try{
if(document.querySelector('[data-reactroot],[data-reactid]')||w.__REACT_DEVTOOLS_GLOBAL_HOOK__&&w.__REACT_DEVTOOLS_GLOBAL_HOOK__.renderers&&w.__REACT_DEVTOOLS_GLOBAL_HOOK__.renderers.size||[...document.querySelectorAll('body *')].slice(0,400).some(e=>Object.keys(e).some(k=>k.startsWith('__reactFiber$')||k.startsWith('__reactContainer$'))))add('React',w.React&&w.React.version);
if(w.__NEXT_DATA__||document.querySelector('#__next'))add('Next.js',w.next&&w.next.version);
if(w.__NUXT__||w.$nuxt)add('Nuxt');
if(w.Vue||w.__VUE__||document.querySelector('[data-v-app]')||[...document.querySelectorAll('body *')].slice(0,400).some(e=>e.__vue__||e.__vue_app__))add('Vue',w.Vue&&w.Vue.version);
const ng=document.querySelector('[ng-version]');if(ng)add('Angular',ng.getAttribute('ng-version'));else if(w.angular)add('AngularJS',w.angular.version&&w.angular.version.full);
if(document.querySelector('[class*=svelte-]'))add('Svelte');
if(w.jQuery)add('jQuery',w.jQuery.fn&&w.jQuery.fn.jquery);
if(w.htmx)add('htmx',w.htmx.version);
if(w.Alpine)add('Alpine.js',w.Alpine.version);
if(document.querySelector('astro-island'))add('Astro');
if(w.Ember)add('Ember',w.Ember.VERSION);
if(w.Backbone)add('Backbone',w.Backbone.VERSION);
if(w._&&w._.VERSION)add('lodash/underscore',w._.VERSION);
if(document.querySelector('[data-lit],lit-element')||customElements&&document.querySelector('*:not(:defined)'))add('web components');
if(Object.keys(w).some(k=>/^webpackChunk|^webpackJsonp/.test(k)))add('webpack');
if(document.querySelector('script[src*="/@vite/"],script[type=module][src*="/src/"]'))add('Vite (dev)');
if(w.Shopify)add('Shopify');
if(w.wp||document.querySelector('meta[name=generator][content*=WordPress]'))add('WordPress');
if(w.gtag||w.ga||w.dataLayer)add('analytics (gtag/ga/dataLayer)');
if(w.Sentry||w.__SENTRY__)add('Sentry');
if(w.bootstrap||document.querySelector('link[href*=bootstrap]'))add('Bootstrap');
if(w.tailwind||[...document.querySelectorAll('[class]')].slice(0,300).filter(e=>/\b(flex|grid|px-\d|py-\d|text-(sm|lg|xl)|bg-\w+-\d00)\b/.test(e.className)).length>30)add('Tailwind CSS (likely)');
const g=document.querySelector('meta[name=generator]');if(g)add('generator',g.content);
}catch(e){f.push('(detection error: '+e.message+')')}
return {frameworks:f,service_worker:!!(navigator.serviceWorker&&navigator.serviceWorker.controller),webmcp:!!navigator.modelContext};})()"#;

/// Storage dump: sizes and clipped values.
pub const STORAGE: &str = r#"(async(a)=>{
const dump=(s)=>{const o=[];for(let i=0;i<s.length&&i<(a.limit||100);i++){const k=s.key(i);const v=s.getItem(k);o.push({k:k,len:v.length,v:v.slice(0,a.value_chars||120)});}return {count:s.length,items:o};};
const r={origin:location.origin};
try{r.local=dump(localStorage)}catch(e){r.local={error:e.message}}
try{r.session=dump(sessionStorage)}catch(e){r.session={error:e.message}}
try{if(indexedDB.databases)r.indexeddb=(await indexedDB.databases()).map(d=>d.name+' v'+d.version)}catch(e){r.indexeddb={error:e.message}}
try{if(self.caches)r.caches=await caches.keys()}catch(e){r.caches=[]}
try{if(navigator.serviceWorker){const regs=await navigator.serviceWorker.getRegistrations();r.service_workers=regs.map(g=>({scope:g.scope,script:(g.active||g.waiting||g.installing||{}).scriptURL||'',state:(g.active||g.waiting||g.installing||{}).state||''}))}}catch(e){r.service_workers=[]}
try{if(navigator.storage&&navigator.storage.estimate){const e=await navigator.storage.estimate();r.quota={usage:e.usage,quota:e.quota}}}catch(e){}
r.cookie_names_visible_to_js=document.cookie?document.cookie.split(';').map(c=>c.split('=')[0].trim()):[];
return r;})({ARGS})"#;

/// WebMCP: the page's declared tools (declarative attributes, links, well-known hints) and the recorder.
pub const WEBMCP_SCAN: &str = r#"(()=>{
const tools=[];
for(const f of document.querySelectorAll('form[toolname]')){
 const params={};for(const el of f.querySelectorAll('input,select,textarea')){if(!el.name)continue;params[el.name]={type:el.type==='number'?'number':el.type==='checkbox'?'boolean':'string',description:el.getAttribute('toolparamdescription')||el.title||el.placeholder||''};}
 tools.push({name:f.getAttribute('toolname'),description:f.getAttribute('tooldescription')||'',source:'declarative-form',inputSchema:{type:'object',properties:params}});}
const rec=window.__adWebMCP;
if(rec)for(const [n,t] of rec.tools)tools.push({name:n,description:t.description||'',source:'imperative',inputSchema:t.inputSchema||{type:'object'}});
const links=[...document.querySelectorAll('link[rel=mcp],link[rel="mcp-server"],script[type="application/mcp+json"]')].map(e=>e.href||e.src||(e.textContent||'').slice(0,200));
return {native_api:!!navigator.modelContext,recorder:!!rec,tools:tools,links:links,wellknown_hint:'/.well-known/mcp.json'};})()"#;

/// Installed on new documents when WebMCP watching is armed: records tools registered through navigator.modelContext.
pub const WEBMCP_SHIM: &str = r#"(()=>{
if(window.__adWebMCP)return;
const rec={tools:new Map()};Object.defineProperty(window,'__adWebMCP',{value:rec,enumerable:false});
const wrap=(mc)=>{
 const rt=mc.registerTool&&mc.registerTool.bind(mc);
 mc.registerTool=function(t){try{rec.tools.set(t.name,t)}catch(e){};return rt?rt(t):undefined};
 const pc=mc.provideContext&&mc.provideContext.bind(mc);
 if(pc)mc.provideContext=function(c){try{rec.tools.clear();(c&&c.tools||[]).forEach(t=>rec.tools.set(t.name,t))}catch(e){};return pc(c)};
 const ut=mc.unregisterTool&&mc.unregisterTool.bind(mc);
 if(ut)mc.unregisterTool=function(n){rec.tools.delete(n);return ut(n)};};
if(navigator.modelContext)wrap(navigator.modelContext);
else{const mc={registerTool(t){rec.tools.set(t.name,t)},unregisterTool(n){rec.tools.delete(n)},provideContext(c){rec.tools.clear();(c&&c.tools||[]).forEach(t=>rec.tools.set(t.name,t))}};
 try{Object.defineProperty(navigator,'modelContext',{value:mc,configurable:true})}catch(e){}}
})()"#;

/// Call a recorded WebMCP tool, or submit a declarative one.
pub const WEBMCP_CALL: &str = r#"(async(a)=>{
const rec=window.__adWebMCP;
const t=rec&&rec.tools.get(a.name);
if(t){const r=await t.execute(a.args||{},{requestUserInteraction:async(f)=>f&&f()});return {source:'imperative',result:r};}
const f=document.querySelector('form[toolname="'+a.name.replace(/"/g,'')+'"]');
if(f){for(const [k,v] of Object.entries(a.args||{})){const el=f.elements[k];if(!el)continue;if(el.type==='checkbox')el.checked=!!v;else el.value=String(v);el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));}
 if(f.requestSubmit)f.requestSubmit();else f.submit();return {source:'declarative-form',result:'submitted'};}
return {error:'no such tool '+a.name};})({ARGS})"#;

/// Replay a request from inside the page (same cookies and origin).
pub const REPLAY: &str = r#"(async(a)=>{
const init={method:a.method||'GET',headers:a.headers||{},credentials:'include'};
if(a.body!==undefined&&init.method!=='GET'&&init.method!=='HEAD')init.body=a.body;
const t0=performance.now();
const r=await fetch(a.url,init);
const text=await r.text();
const h={};r.headers.forEach((v,k)=>{h[k]=v.slice(0,200)});
return {status:r.status,ok:r.ok,ms:Math.round(performance.now()-t0),headers:h,bytes:text.length,body:text.slice(0,a.max||2000)};})({ARGS})"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// `ATLAS_DRIVE_DUMP_JS=dir cargo test dump_scripts` writes every script (ARGS = {}) for `node --check`.
    #[test]
    fn dump_scripts() {
        let Some(dir) = std::env::var_os("ATLAS_DRIVE_DUMP_JS") else {
            return;
        };
        for (n, s) in [
            ("focus", FOCUS),
            ("wait", WAIT),
            ("links", LINKS),
            ("forms", FORMS),
            ("traps", TRAPS),
            ("extract", EXTRACT),
            ("outline", OUTLINE),
            ("frameworks", FRAMEWORKS),
            ("storage", STORAGE),
            ("webmcp_scan", WEBMCP_SCAN),
            ("webmcp_shim", WEBMCP_SHIM),
            ("webmcp_call", WEBMCP_CALL),
            ("replay", REPLAY),
        ] {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("{n}.js")),
                with_args(s, &serde_json::json!({})),
            )
            .unwrap();
        }
    }

    #[test]
    fn args_are_substituted_once_per_marker() {
        let s = with_args(WAIT, &serde_json::json!({"kind":"text","value":"x"}));
        assert!(s.contains(r#"{"kind":"text","value":"x"}"#));
        assert!(!s.contains("{ARGS}"));
        for sc in [
            LINKS,
            FORMS,
            TRAPS,
            EXTRACT,
            OUTLINE,
            STORAGE,
            WEBMCP_CALL,
            REPLAY,
            WAIT,
        ] {
            assert_eq!(sc.matches("{ARGS}").count(), 1);
            // balanced braces and parentheses (a cheap syntax sanity check; the forge tests run them for real)
            let b = |o: char, c: char| {
                sc.chars().filter(|x| *x == o).count() == sc.chars().filter(|x| *x == c).count()
            };
            assert!(b('(', ')') && b('{', '}') && b('[', ']'));
        }
    }
}
