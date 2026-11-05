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

/// The in-page accessibility mirror (src/mirror.rs, ADR 0063 round 2): one walk of the DOM in Drive's
/// isolated world, the page cannot see or change it. Rows of 8: [element id, parent row, role, name,
/// state letters, value, url, heading level]; element ids are stable while the document lives
/// (a WeakMap in the isolated world), so refs survive re-renders. Hidden-by-a-trick and
/// instruction-like text is withheld here and counted. An element of opacity 0 is still in the tree (a styled
/// checkbox is often an invisible input under its label); only its own text counts as hidden.
pub const SNAP: &str = r#"((a)=>{
const W=globalThis.__ad||(globalThis.__ad={m:new WeakMap(),els:[null]});
const idOf=(el)=>{let i=W.m.get(el);if(!i){i=W.els.length;W.els.push(el);W.m.set(el,i);}return i;};
const INJ=/(ignore|disregard|forget|override)\b[^.]{0,40}\b(previous|prior|above|earlier|all|any)\b[^.]{0,30}\b(instruction|prompt|rule|message)s?|system prompt|you are (now |actually )?(an? |the )?(ai|assistant|model|llm|agent|chatbot)|as an ai\b|(do not|don't|never) (tell|reveal|mention|inform|show)\b[^.]{0,30}\b(user|human|person)|<\|im_(start|end)\|>|\[\/?INST\]|###\s*(system|instruction)|(send|post|forward|exfiltrate|email|upload)\b[^.]{0,60}\b(password|token|cookie|secret|credential|api[ _-]?key|conversation|chat history|private)|new instructions?:|important:?\s*(ai|assistant|agent|llm)|(ai|llm|assistant|agent)s?,?\s+(must|should|please|need to)\b/i;
const SKIP={SCRIPT:1,STYLE:1,NOSCRIPT:1,TEMPLATE:1,HEAD:1,TITLE:1,META:1,LINK:1,IFRAME:1,OBJECT:1,EMBED:1,CANVAS:0,DATALIST:1,PARAM:1,SOURCE:1,TRACK:1,BASE:1};
const TAG={NAV:'navigation',MAIN:'main',ASIDE:'complementary',UL:'list',OL:'list',MENU:'list',LI:'listitem',TABLE:'table',THEAD:'rowgroup',TBODY:'rowgroup',TFOOT:'rowgroup',TR:'row',TD:'cell',P:'paragraph',DIALOG:'dialog',FIGURE:'figure',BLOCKQUOTE:'blockquote',PRE:'Pre',ARTICLE:'article',FIELDSET:'group',DETAILS:'group',OUTPUT:'status',PROGRESS:'progressbar',METER:'meter',TEXTAREA:'textbox',SEARCH:'search',DL:'DescriptionList',DT:'DescriptionListTerm',DD:'DescriptionListDetail',CODE:'code',EM:'emphasis',STRONG:'strong',SUB:'subscript',SUP:'superscript',MARK:'mark',TIME:'time',ABBR:'Abbr',HR:'separator',OPTION:'option',OPTGROUP:'group',SUMMARY:'button',LABEL:'',SVG:''};
const INPUT={button:'button',submit:'button',reset:'button',image:'button',file:'button',checkbox:'checkbox',radio:'radio',range:'slider',number:'spinbutton',search:'searchbox',color:'colorwell',hidden:''};
const NAMEFROM={button:1,link:1,heading:1,tab:1,option:1,menuitem:1,menuitemcheckbox:1,menuitemradio:1,treeitem:1,checkbox:1,radio:1,switch:1,tooltip:1,cell:1,columnheader:1,rowheader:1,gridcell:1,row:0};
const vw=innerWidth,vh=innerHeight,geo=!!a.geo,safe=a.safe!==false;
const out=[];const act=document.activeElement;let withheld=0;
const sq=(t)=>t.replace(/\s+/g,' ').trim();
const txt=(el)=>sq(el.textContent||'');
const rgb=(c)=>{const m=c.match(/[\d.]+/g);return m?m.map(Number):null;};
const bgm=new Map();
const bgOf=(el)=>{const ch=[];let c=null;for(let e=el;e;e=e.parentElement){const k=bgm.get(e);if(k){c=k;break;}ch.push(e);const v=rgb(getComputedStyle(e).backgroundColor);if(v&&(v.length<4||v[3]>0.1)){c=v;break;}}c=c||[255,255,255,1];for(const e of ch)bgm.set(e,c);return c;};
const lum=(c)=>{const f=(v)=>{v/=255;return v<=0.03928?v/12.92:Math.pow((v+0.055)/1.055,2.4)};return 0.2126*f(c[0])+0.7152*f(c[1])+0.0722*f(c[2]);};
const trap=(el)=>{if(!el.checkVisibility({opacityProperty:true}))return true;const cs=getComputedStyle(el);if(parseFloat(cs.opacity)<0.05||parseFloat(cs.fontSize)<2)return true;const fg=rgb(cs.color);if(!fg||(fg.length>3&&fg[3]<=0.1))return false;const bg=bgOf(el);const l1=lum(fg),l2=lum(bg);return (Math.max(l1,l2)+0.05)/(Math.min(l1,l2)+0.05)<1.15;};
const tm=new Map();const hid=(el)=>{let v=tm.get(el);if(v===undefined){v=trap(el);tm.set(el,v);}return v;};
const byIds=(ids)=>sq(ids.split(/\s+/).map((i)=>{const e=document.getElementById(i);return e?(e.getAttribute('aria-label')||e.textContent||''):''}).join(' '));
function roleOf(el,t){
 const r=el.getAttribute('role');if(r){const f=r.trim().split(/\s+/)[0];if(f)return f==='none'?'presentation':f;}
 switch(t){
 case 'A':case 'AREA':return el.hasAttribute('href')?'link':'';
 case 'BUTTON':return 'button';
 case 'INPUT':{const ty=(el.getAttribute('type')||'text').toLowerCase();if(ty in INPUT)return INPUT[ty];return el.hasAttribute('list')?'combobox':'textbox';}
 case 'SELECT':return (el.multiple||el.size>1)?'listbox':'combobox';
 case 'H1':case 'H2':case 'H3':case 'H4':case 'H5':case 'H6':return 'heading';
 case 'IMG':return el.getAttribute('alt')===''?'presentation':'img';
 case 'HEADER':return el.closest('article,aside,main,nav,section,[role=article],[role=complementary],[role=main],[role=navigation],[role=region]')?'':'banner';
 case 'FOOTER':return el.closest('article,aside,main,nav,section,[role=article],[role=complementary],[role=main],[role=navigation],[role=region]')?'':'contentinfo';
 case 'SECTION':return (el.hasAttribute('aria-label')||el.hasAttribute('aria-labelledby'))?'region':'';
 case 'FORM':return (el.hasAttribute('aria-label')||el.hasAttribute('aria-labelledby'))?'form':'';
 case 'TH':return (el.scope==='row'||(!el.scope&&el.parentElement&&el.parentElement.firstElementChild===el&&el.parentElement.querySelector('td')))?'rowheader':'columnheader';
 case 'DIALOG':return el.open?'dialog':'';
 }
 const x=TAG[t];return x===undefined?'':x;
}
function nameOf(el,role,t){
 let v=el.getAttribute('aria-labelledby');if(v){v=byIds(v);if(v)return v;}
 v=el.getAttribute('aria-label');if(v&&v.trim())return sq(v);
 if(t==='INPUT'||t==='SELECT'||t==='TEXTAREA'){
  const ty=(el.type||'').toLowerCase();
  if(ty==='button'||ty==='submit'||ty==='reset')return el.value||(ty==='submit'?'Submit':ty==='reset'?'Reset':'');
  if(ty==='image')return el.alt||'Submit';
  if(el.labels&&el.labels.length){const s=sq([...el.labels].map((l)=>l.textContent).join(' '));if(s)return s;}
  return sq(el.getAttribute('placeholder')||el.title||'');
 }
 if(t==='IMG'||t==='AREA')return sq(el.getAttribute('alt')||el.title||'');
 if(t==='FIELDSET'){const l=el.querySelector(':scope>legend');if(l)return txt(l);}
 if(t==='FIGURE'){const l=el.querySelector(':scope>figcaption');if(l)return txt(l);}
 if(t==='TABLE'){const c=el.caption;if(c)return txt(c);}
 if(NAMEFROM[role]){
  if(!sq(el.textContent||'')){const i=el.querySelector('img[alt],[aria-label]');if(i)return sq(i.getAttribute('alt')||i.getAttribute('aria-label')||'');return sq(el.title||'');}
  return '';
 }
 if(role==='dialog'||role==='region'||role==='navigation'||role==='form'||role==='group'||role==='img')return sq(el.title||'');
 return '';
}
function flags(el,role,t){
 let f='';const g=(n)=>el.getAttribute(n);
 if(el.disabled||g('aria-disabled')==='true')f+='d';
 const ck=g('aria-checked');
 if(role==='checkbox'||role==='radio'||role==='switch'||role==='menuitemcheckbox'||role==='menuitemradio'){
  if(ck==='mixed'||el.indeterminate)f+='m';else if(ck==='true'||(ck===null&&el.checked))f+='c';else f+='u';}
 if(g('aria-selected')==='true'||(t==='OPTION'&&el.selected))f+='s';
 const ex=g('aria-expanded');if(ex==='true')f+='e';else if(ex==='false')f+='x';
 if(t==='SUMMARY'&&el.parentElement&&el.parentElement.tagName==='DETAILS')f+=el.parentElement.open?'e':'x';
 if(el.required||g('aria-required')==='true')f+='r';
 if((el.readOnly&&(t==='INPUT'||t==='TEXTAREA'))||g('aria-readonly')==='true')f+='o';
 if(g('aria-invalid')==='true')f+='i';
 if(g('aria-pressed')==='true')f+='p';
 if(g('aria-modal')==='true'||(t==='DIALOG'&&el.matches(':modal')))f+='M';
 if(el===act)f+='f';
 return f;
}
const LINE={button:1,link:1,textbox:1,searchbox:1,combobox:1,checkbox:1,radio:1,switch:1,slider:1,spinbutton:1,menuitem:1,tab:1,option:0,treeitem:1,heading:1,navigation:1,main:1,banner:1,contentinfo:1,complementary:1,region:1,form:1,dialog:1,alertdialog:1,table:1,search:1};
function push(id,par,role,name,f,val,url,lvl){out.push(id,par,role,name,f,val,url,lvl);return out.length/8-1;}
function kids(el,par,depth){
 const root=el.shadowRoot||el;
 let ch=root.firstChild;
 if(el.tagName==='SLOT'){const as=el.assignedNodes({flatten:true});if(as.length){for(const c of as)node(c,par,depth);return;}}
 for(;ch;ch=ch.nextSibling)node(ch,par,depth);
}
function node(c,par,depth){
 if(depth>400)return;
 if(c.nodeType===3){const d=c.data;if(!d.trim())return;const s=d.replace(/\s+/g,' ');
  if(safe&&s.length>=12&&(INJ.test(s)||(c.parentElement&&hid(c.parentElement)))){withheld++;return;}
  push(c.parentElement?idOf(c.parentElement):0,par,'StaticText',s,'','','',0);return;}
 if(c.nodeType!==1)return;
 const t=c.tagName.toUpperCase();
 if(SKIP[t])return;
 if(c.getAttribute('aria-hidden')==='true'||c.hasAttribute('inert')||c.hidden&&t!=='DETAILS'){return;}
 if(t==='BR'){push(0,par,'LineBreak','','','','',0);return;}
 if(!c.checkVisibility({visibilityProperty:true})){
  if(t==='OPTION'||t==='OPTGROUP'){}
  else{const cs=getComputedStyle(c);if(cs.display!=='contents')return;}
 }
 let role=roleOf(c,t);
 if(role==='presentation'||role==='none'||role==='generic')role='';
 if(t==='INPUT'&&role===''&&(c.type||'')==='hidden')return;
 if(!role){
  // text that is invisible by a trick, or reads like an instruction, is withheld where it is read (node, text branch)
  kids(c,par,depth+1);return;
 }
 let name=nameOf(c,role,t);
 if(name.length>400)name=name.slice(0,400);
 if(safe&&(role==='img'||role==='status'||role==='alert')&&name.length>=12&&INJ.test(name)){withheld++;return;}
 let f=flags(c,role,t);
 if(geo&&LINE[role]){const r=c.getBoundingClientRect();if(r.right>0&&r.bottom>0&&r.left<vw&&r.top<vh&&(r.width>0||r.height>0))f+='v';}
 let val='';
 if(t==='INPUT'||t==='TEXTAREA'){if(role!=='button'&&role!=='checkbox'&&role!=='radio')val=c.type==='password'?'•'.repeat(Math.min(c.value.length,8)):c.value;}
 else if(t==='SELECT'){const o=c.selectedOptions&&c.selectedOptions[0];val=o?sq(o.textContent):'';}
 else if(role==='slider'||role==='spinbutton'||role==='progressbar')val=c.getAttribute('aria-valuenow')||'';
 const lvl=role==='heading'?(+(c.getAttribute('aria-level')||(/^H[1-6]$/.test(t)?t[1]:2))):0;
 const url=role==='link'&&c.href?String(c.href):'';
 const me=push(idOf(c),par,role,name,f,val,url,lvl);
 if(t==='SELECT'){for(const o of c.options)push(idOf(o),me,'option',sq(o.label||o.textContent),o.selected?'s':'','','',0);return;}
 if(t==='INPUT'||t==='TEXTAREA')return;
 kids(c,me,depth+1);
}
push(0,-1,'RootWebArea',document.title||'','','','',0);
if(document.body)kids(document.body,0,0);
return {rows:out,withheld:withheld,n:W.els.length};
})({ARGS})"#;

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
