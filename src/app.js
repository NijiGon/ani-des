const{invoke}=window.__TAURI__.core;
let S={view:'home',mode:'sub',country:'ALL',query:'',results:[],episodes:[],anime:null,details:null,links:[],history:[],bookmarks:[],autoNext:true,currentEp:'',viewMode:'grid',libTag:''};
let saveInt=null,progAnim=null,acTimer=null,dlRef=null;
let navHistory=[],navIndex=-1,navLock=false;

// Escaping: API data is untrusted. Every value interpolated into markup goes through esc().
function esc(s){return String(s??'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;');}

// Event wiring: markup carries data-act / data-change / data-input plus JSON args; one listener per event type handles them.
function A(act,args=[]){return `data-act="${esc(act)}" data-args="${esc(JSON.stringify(args))}"`;}
function C(name,args=[]){return `data-change="${esc(name)}" data-args="${esc(JSON.stringify(args))}"`;}
function I(name,args=[]){return `data-input="${esc(name)}" data-args="${esc(JSON.stringify(args))}"`;}

// Element and event of the control that fired the current action.
let ctx={el:null,ev:null};
function argsOf(el){try{return JSON.parse(el.dataset.args||'[]');}catch(e){return [];}}

document.addEventListener('click',e=>{
  const el=e.target.closest('[data-act]');
  if(!el||e.target.closest('[data-nostop]'))return;
  const fn=ACTIONS[el.dataset.act];if(!fn)return;
  ctx={el,ev:e};fn(...argsOf(el));
});
document.addEventListener('change',e=>{
  const el=e.target.closest('[data-change]');if(!el)return;
  const fn=ACTIONS[el.dataset.change];if(!fn)return;
  ctx={el,ev:e};fn(...argsOf(el));
});
document.addEventListener('input',e=>{
  const el=e.target.closest('[data-input]');if(!el)return;
  const fn=ACTIONS[el.dataset.input];if(!fn)return;
  ctx={el,ev:e};fn(...argsOf(el));
});
// Broken images: fall back to a placeholder, or ask the server for a thumbnail once.
document.addEventListener('error',e=>{
  const img=e.target;
  if(!(img instanceof HTMLImageElement))return;
  if(img.dataset.err==='thumb')tryThumbFallback(img);
  else if(img.dataset.err==='ph')img.outerHTML=cardPh();
},true);

function pushNav(){
  if(navLock)return;
  const html=document.getElementById('app')?.innerHTML||'';
  const snap={view:S.view,anime:S.anime?JSON.parse(JSON.stringify(S.anime)):null,details:S.details?JSON.parse(JSON.stringify(S.details)):null,episodes:[...S.episodes],results:S.results,query:S.query,currentEp:S.currentEp,html,scrollY:window.scrollY};
  navHistory=navHistory.slice(0,navIndex+1);
  navHistory.push(snap);navIndex=navHistory.length-1;
}
function updateNavSnap(){if(navIndex>=0&&navHistory[navIndex]){navHistory[navIndex].html=document.getElementById('app')?.innerHTML||'';navHistory[navIndex].scrollY=window.scrollY;}}
function navBack(){
  if(navIndex<=0)return;updateNavSnap();navIndex--;navLock=true;
  restoreSnap(navHistory[navIndex]);
}
function navForward(){
  if(navIndex>=navHistory.length-1)return;updateNavSnap();navIndex++;navLock=true;
  restoreSnap(navHistory[navIndex]);
}
function restoreSnap(snap){
  S.view=snap.view;S.anime=snap.anime;S.details=snap.details;S.episodes=snap.episodes;S.results=snap.results;S.query=snap.query;S.currentEp=snap.currentEp;
  const app=document.getElementById('app');
  if(app&&snap.html)app.innerHTML=snap.html;
  requestAnimationFrame(()=>requestAnimationFrame(()=>window.scrollTo(0,snap.scrollY||0)));
  navLock=false;
}

async function uc(){try{const s=await invoke('get_cache_stats');document.getElementById('cbadge').textContent=`${s.search_entries+s.episode_entries+s.link_entries} cached`;}catch(e){}}

function skelCards(n){return Array(n).fill('<div class="card"><div class="skel skel-card"></div><div class="card-body"><div class="skel skel-text" style="width:80%"></div><div class="skel skel-text" style="width:50%"></div></div></div>').join('');}

function modeButtons(search){return ['sub','dub'].map(m=>`<button class="btn bg ${S.mode===m?'active':''}" ${A('setMode',[m,search])}>${m==='sub'?'Sub':'Dub'}</button>`).join('');}
function countryButtons(search){return ['ALL','JP','CN','KR'].map(c=>`<button class="btn bg ${S.country===c?'active':''}" ${A('setCountry',[c,search])}>${c}</button>`).join('');}
function filterHtml(search){return `<span class="flabel">Mode</span><div class="fgroup">${modeButtons(search)}</div><span class="flabel" style="margin-left:8px">Origin</span><div class="fgroup">${countryButtons(search)}</div>`;}
function renderFilters(search){const f=document.querySelector('.filters');if(f)f.innerHTML=filterHtml(search);}

function bindSearchBox(){
  const q=document.getElementById('q');if(!q)return;
  q.addEventListener('keydown',e=>{if(e.key==='Enter')doSearch();});
  q.addEventListener('input',onSearchInput);
  q.addEventListener('blur',()=>setTimeout(()=>document.getElementById('aclist')?.classList.remove('show'),150));
}

function render(){
  clearInterval(saveInt);uc();const app=document.getElementById('app');

  if(S.view==='home'){
    app.innerHTML=`
      <div style="display:flex;gap:8px;margin-bottom:8px">
        <div class="search-wrap"><input class="sinput" id="q" value="${esc(S.query)}" placeholder="Search anime..." autofocus/><div class="ac-list" id="aclist"></div></div>
        <button class="btn bp" ${A('doSearch')}>Search</button>
      </div>
      <div class="filters">${filterHtml(false)}</div>
      <div id="hist"></div>
      <div id="recent"><div class="section-header"><span class="slabel">Currently Airing</span></div><div class="cards">${skelCards(6)}</div></div>`;
    bindSearchBox();
    loadHistory();if(!navLock)loadHome();

  }else if(S.view==='search'){
    app.innerHTML=`
      <div style="display:flex;gap:8px;margin-bottom:10px">
        <div class="search-wrap"><input class="sinput" id="q" value="${esc(S.query)}" placeholder="Search anime..."/><div class="ac-list" id="aclist"></div></div>
        <button class="btn bp" ${A('doSearch')}>Search</button>
        <button class="btn bg" ${A('openSaveBookmark')}><i data-lucide="bookmark-plus" style="width:14px;height:14px"></i></button>
      </div>
      <div class="filters">${filterHtml(true)}</div>
      <div id="status" class="status-msg"></div>
      <div class="cards" id="results">${S.results.length?'':skelCards(8)}</div>`;
    bindSearchBox();
    if(S.results.length)renderCards();

  }else if(S.view==='detail'){
    const d=S.details;
    if(!d){app.innerHTML=bc()+`<div style="margin-top:16px">${skelCards(1)}</div>`;return;}
    app.innerHTML=bc()+`
      <div class="detail" style="margin-top:10px">
        <div class="detail-thumb">${d.thumbnail?`<img src="${esc(d.thumbnail)}" data-err="ph"/>`:'<div style="height:180px" class="skel"></div>'}</div>
        <div class="detail-info">
          <div class="detail-title">${esc(d.name)}</div>
          ${d.title_romaji&&d.title_romaji!==d.name?`<div style="font-size:10px;color:var(--text2);margin-bottom:5px">${esc(d.title_romaji)}</div>`:''}
          <div class="detail-meta">
            ${d.status?`<span class="tag st">${esc(d.status)}</span>`:''}
            <span class="tag">${d.episode_count||S.episodes.length} ep</span>
            ${d.rating?`<span class="tag" style="color:#ffd700;border-color:#ffd700">★ ${d.rating.toFixed(1)}</span>`:''}
            ${(d.genres||[]).map(g=>`<span class="tag">${esc(g)}</span>`).join('')}
          </div>
          <div class="detail-desc">${esc(cleanDesc(d.description))}</div>
          <div style="margin-top:8px;display:flex;gap:6px"><button class="btn bp" ${A('watch')}>▶ Watch</button><select class="sinput" style="width:auto;padding:5px 8px;font-size:10px" ${C('addToLibrary')}><option value="">+ Library</option><option value="watching">Watching</option><option value="plan_to_watch">Plan to Watch</option><option value="finished">Finished</option><option value="dropped">Dropped</option></select></div>
        </div>
      </div>
      <div class="slabel" style="display:flex;justify-content:space-between;align-items:center"><span>Episodes</span><div style="display:flex;gap:4px"><button class="btn bg" ${A('toggleBulk')} id="bulkbtn">Select</button><button class="btn bp" id="dlselbtn" style="display:none" ${A('downloadSelected')}>↓ DL</button></div></div>
      <div class="epgrid" id="epgrid">${S.episodes.length?S.episodes.map(ep=>`<button class="btn bg ep-btn" data-ep="${esc(ep)}" ${A('epClick',[ep])}>${esc(ep)}</button>`).join(''):Array(12).fill('<div class="skel" style="width:100%;height:30px;border-radius:4px"></div>').join('')}</div>
      <div id="status" class="status-msg"></div>`;

  }else if(S.view==='loading'){
    app.innerHTML=bc()+`<div class="ep-now">${esc(S.anime.name)} — Ep ${esc(S.currentEp)}</div><div class="loader"><div class="lbar"><div class="lfill" id="pbar"></div></div><div class="ltxt" id="ptxt">Fetching sources...</div></div>`;
    animProg();

  }else if(S.view==='player'){
    const idx=S.episodes.indexOf(S.currentEp);
    app.innerHTML=bc()+`
      <div class="player-layout" style="margin-top:8px">
        <div class="player-main">
          <div class="ep-now">Episode ${esc(S.currentEp)}</div>
          <div class="player-wrap" id="pwrap">
            <video id="vp" ${A('togglePlay')}></video>
            <button class="skip-btn hidden" id="skipbtn" ${A('doSkip')}>Skip ▶</button>
            <div class="ctrls">
              <div class="seekbar" id="seekbar" ${A('seek')}><div class="seek-buf" id="seekbuf"></div><div class="seek-prog" id="seekprog"></div></div>
              <div class="crow">
                <div class="cleft">
                  <button class="cbtn" id="playbtn" ${A('togglePlay')}><svg viewBox="0 0 24 24"><path id="playico" d="M8 5v14l11-7z"/></svg></button>
                  <button class="cbtn" ${A('skip',[-5])} title="Back 5s"><svg viewBox="0 0 24 24"><path d="M12.5 8c-2.65 0-5.05.99-6.9 2.6L2 7v9h9l-3.62-3.62c1.39-1.16 3.16-1.88 5.12-1.88 3.54 0 6.55 2.31 7.6 5.5l2.37-.78C21.08 11.03 17.15 8 12.5 8z"/></svg></button>
                  <button class="cbtn" ${A('skip',[5])} title="Forward 5s"><svg viewBox="0 0 24 24"><path d="M11.5 8c2.65 0 5.05.99 6.9 2.6L22 7v9h-9l3.62-3.62c-1.39-1.16-3.16-1.88-5.12-1.88-3.54 0-6.55 2.31-7.6 5.5l-2.37-.78C2.92 11.03 6.85 8 11.5 8z"/></svg></button>
                  <span class="ctime" id="ctime">0:00 / 0:00</span>
                </div>
                <div class="cright">
                  <div class="vol-wrap"><button class="cbtn" id="volbtn" ${A('toggleMute')}><svg viewBox="0 0 24 24"><path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"/></svg></button><input type="range" class="vol-s" min="0" max="1" step=".05" value="1" ${I('setVol')}/></div>
                  <button class="spd" ${A('cycleSpeed')} id="speedbtn">1x</button>
                  <button class="cbtn" ${A('toggleFS')} title="Fullscreen"><svg viewBox="0 0 24 24"><path d="M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z"/></svg></button>
                </div>
              </div>
            </div>
          </div>
          <div class="pbar">
            <div class="left"><button class="btn bg" ${idx>0?'':'disabled'} ${A('prevEp')}>◀ Prev</button><button class="btn bp" ${idx<S.episodes.length-1?'':'disabled'} ${A('nextEp')}>Next ▶</button></div>
            <div class="right"><label class="toggle"><input type="checkbox" ${S.autoNext?'checked':''} ${C('setAutoNext')}>Auto-next</label></div>
          </div>
          <div class="qrow">${S.links.map((l,i)=>`<button class="btn ${i===0?'bp':'bg'} qbtn" ${A('playSrc',[i])}>${esc(l.quality||'default')}</button>`).join('')}</div>
          <div id="status" class="status-msg">${S.links.length?'':`No sources. <button class="btn bg" ${A('debugSources')} style="margin-left:6px">Debug</button>`}</div>
        </div>
        <div class="player-sidebar"><div class="slabel" style="margin-top:0">Episodes</div>${S.episodes.map(ep=>`<div class="ep-item ${ep===S.currentEp?'active':''}" ${A('getLinks',[ep])}>Ep ${esc(ep)}</div>`).join('')}</div>
      </div>`;
    if(S.links.length)playSrc(0);

  }else if(S.view==='library'){
    const labels={watching:'Watching',plan_to_watch:'Plan to Watch','break':'On Break',finished:'Finished',dropped:'Dropped'};
    app.innerHTML=bc()+`
      <div class="filters" style="margin-bottom:12px" id="lib-filters">${Object.entries(labels).map(([t,l])=>`<button class="btn bg ${S.libTag===t?'active':''}" ${A('setLibTag',[t])}>${l}</button>`).join('')}<button class="btn bg ${!S.libTag?'active':''}" ${A('setLibTag',[''])}>All</button></div>
      <div id="lib-content"></div>`;
    loadLibrary();

  }else if(S.view==='bookmarks'){
    app.innerHTML=bc()+`<div class="slabel">Bookmarks</div><ul class="list">${S.bookmarks.map((b,i)=>`<li class="li" ${A('loadBm',[i])}><div><span class="t">${esc(b.name)}</span><br><span style="font-size:9px;color:var(--text2)">${esc(b.query)} · ${esc(b.mode)} · ${esc(b.country)}</span></div><span class="del" ${A('delBm',[b.id])}>✕</span></li>`).join('')}</ul>${S.bookmarks.length?'':'<div class="status-msg">No bookmarks</div>'}`;

  }else if(S.view==='downloads'){
    app.innerHTML=bc()+`<div class="slabel" style="display:flex;justify-content:space-between;align-items:center"><span>Downloads</span><button class="btn bg" ${A('openDownloadFolder')}>Open Folder</button></div><ul class="list" id="dl-list"></ul>`;
    refreshDownloads();
  }
  animate();
}

async function loadHistory(){
  try{
    S.history=await invoke('get_history');
    const el=document.getElementById('hist');
    if(!S.history.length){el.innerHTML='';return;}
    el.innerHTML=`<div class="section-header"><span class="slabel">Continue Watching</span></div><div class="cards ${S.viewMode==='list'?'list-view':''}">${S.history.slice(0,6).map(h=>`<div class="card" ${A('resumeH2',[h.id,h.mode])} title="${esc(h.name)}">${h.thumbnail?`<img class="card-img" src="${esc(h.thumbnail)}" data-err="ph"/>`:`${cardPh()}`}<div class="card-body"><div class="card-title">${esc(h.name)}</div><div class="card-meta">Ep ${esc(h.episode)} · ${fmt(h.progress)}</div></div></div>`).join('')}</div>`;
  }catch(e){}
}
function fmt(s){const m=Math.floor(s/60),sec=Math.floor(s%60);return m+':'+(sec<10?'0':'')+sec;}
function fmtBytes(b){if(!b)return'0 B';const u=['B','KB','MB','GB'];let i=0;let v=b;while(v>=1024&&i<3){v/=1024;i++;}return v.toFixed(i?1:0)+' '+u[i];}

function cleanDesc(s){if(!s)return'No description.';return s.replace(/<br\s*\/?>/gi,'\n').replace(/<[^>]*>/g,'').replace(/&amp;/g,'&').replace(/&lt;/g,'<').replace(/&gt;/g,'>').replace(/&quot;/g,'"').replace(/\n{3,}/g,'\n\n').trim();}

function acSelect(i){document.getElementById('aclist')?.classList.remove('show');S.results=window._acResults||[];openDetail(i);}
function onSearchInput(e){
  clearTimeout(acTimer);
  const q=e.target.value.trim();if(q.length<3){document.getElementById('aclist')?.classList.remove('show');return;}
  acTimer=setTimeout(async()=>{
    try{const r=await invoke('search_anime',{query:q,mode:S.mode,country:'ALL'});
    const el=document.getElementById('aclist');if(!el)return;
    if(!r.length){el.classList.remove('show');return;}
    el.innerHTML=r.slice(0,8).map((a,i)=>`<div class="ac-item" ${A('acSelect',[i])}>${esc(a.name)}<span style="color:var(--text2)">(${a.episodes} ep)</span></div>`).join('');
    window._acResults=r.slice(0,8);
    el.classList.add('show');}catch(e){}
  },400);
}

function renderCards(){
  document.getElementById('results').innerHTML=S.results.map((r,i)=>`<div class="card" ${A('openDetail',[i])}>${r.thumbnail?`<img class="card-img" src="${esc(r.thumbnail)}" data-err="ph"/>`:`${cardPh()}`}<div class="card-hover"><div class="ch-title">${esc(r.name)}</div><div class="ch-meta">${r.episodes} ep${r.rating?` · ★${r.rating.toFixed(1)}`:''}</div></div><div class="card-body"><div class="card-title">${esc(r.name)}</div><div class="card-meta">${r.episodes} ep${r.rating?` · ★${r.rating.toFixed(1)}`:''}</div></div></div>`).join('');
}
function cardPh(){return `<div class="card-ph" style="width:100%;aspect-ratio:3/4;background:var(--s2);display:flex;align-items:center;justify-content:center;color:var(--text2)"><svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="2" width="20" height="20" rx="2"/><circle cx="8" cy="8" r="2"/><path d="M21 15l-5-5L5 21"/></svg></div>`;}
function tryThumbFallback(img){if(img.dataset.tried)return;img.dataset.tried='1';invoke('get_thumbnail_fallback',{name:img.dataset.name,malId:Number(img.dataset.mal)||null}).then(url=>{if(url){img.src=url;img.style.display='';}else img.outerHTML=cardPh();}).catch(()=>img.outerHTML=cardPh());}

// Home
async function loadHome(){
  function sec(el,label,items,id){
    if(!el||!items.length)return;
    el.innerHTML=`
      <div class="section-header"><span class="slabel">${label}</span>
        <div class="view-toggle"><button class="btn bg ${S.viewMode==='grid'?'active':''}" ${A('setViewMode',['grid'])}>▦</button><button class="btn bg ${S.viewMode==='list'?'active':''}" ${A('setViewMode',['list'])}>☰</button></div>
      </div>
      <div class="cards ${S.viewMode==='list'?'list-view':''}" id="${id}-cards">${renderItemCards(items)}</div>
      <div style="text-align:center;margin-top:10px"><button class="btn bg" id="${id}-more" ${A('expandSection',[id])}>View more</button></div>`;
    window['_'+id]=items;
    updateNavSnap();
  }
  invoke('get_recently_updated',{mode:S.mode,page:1}).then(i=>sec(document.getElementById('recent'),'Trending',i,'recent')).catch(e=>console.error('recent',e));
}

function renderItemCards(items){
  return items.map(r=>{
    const name=r.title||r.name||'';const img=r.cover||r.thumbnail||'';const eps=r.episodes||0;const score=r.score||r.rating||0;const malId=r.mal_id||0;
    const d={id:r.id?.toString()||'',name,thumbnail:img,episodes:eps,rating:score,mal_id:malId};
    return`<div class="card" data-d="${esc(JSON.stringify(d))}" data-act="openHomeCard">${img?`<img class="card-img" src="${esc(img)}" data-name="${esc(name)}" data-mal="${malId}" data-err="thumb"/>`:`${cardPh()}`}<div class="card-hover"><div class="ch-title">${esc(name)}</div><div class="ch-meta">${eps} ep${score?` · ★${score.toFixed(1)}`:''}</div></div><div class="card-body"><div class="card-title">${esc(name)}</div><div class="card-meta">${eps} ep${score?` · ★${score.toFixed(1)}`:''}</div></div></div>`;
  }).join('');
}

function setViewMode(mode){S.viewMode=mode;document.querySelectorAll('.cards').forEach(c=>c.classList.toggle('list-view',mode==='list'));}

function expandSection(id){
  window['_'+id+'_page']=(window['_'+id+'_page']||1)+1;
  const pg=window['_'+id+'_page'];
  const btn=document.getElementById(id+'-more');
  if(btn){btn.disabled=true;btn.innerHTML='<span class="spinner"></span>';}
  const el=document.getElementById(id+'-cards');
  if(el)el.insertAdjacentHTML('beforeend',skelCards(5));
  invoke('get_recently_updated',{mode:S.mode,page:pg}).then(more=>{
    el?.querySelectorAll('.skel-card')?.forEach(s=>s.closest('.card').remove());
    if(!more.length){if(btn)btn.parentElement.remove();return;}
    window['_'+id]=(window['_'+id]||[]).concat(more);
    if(el)el.insertAdjacentHTML('beforeend',renderItemCards(more));
    if(btn){btn.disabled=false;btn.textContent='View more';}
    updateNavSnap();
  }).catch(()=>{el?.querySelectorAll('.skel-card')?.forEach(s=>s.closest('.card').remove());if(btn){btn.disabled=false;btn.textContent='View more';}});
}

function openHomeCard(){openFromHome(JSON.parse(ctx.el.dataset.d));}
function openFromHome(r){
  S.anime=r;S.results=[r];
  openDetail(0);
}

// Detail
async function openDetail(i){
  S.anime=S.results[i];S.view='detail';S.episodes=[];
  S.details={id:S.anime.id,name:S.anime.name,title_romaji:S.anime.title_romaji||'',thumbnail:S.anime.thumbnail||'',description:'',genres:[],status:'',episode_count:S.anime.episodes,rating:S.anime.rating||0};
  render();pushNav();
  let showId=S.anime.id;
  try{const eps=await invoke('get_episodes',{showId,mode:S.mode});S.episodes=eps;const g=document.getElementById('epgrid');if(g&&S.view==='detail')g.innerHTML=eps.map(ep=>`<button class="btn bg ep-btn" data-ep="${esc(ep)}" ${A('epClick',[ep])}>${esc(ep)}</button>`).join('')||'<div class="status-msg">No episodes</div>';}catch(e){}
  try{const d=await invoke('get_anime_details',{showId});if(S.view==='detail'){S.details={...S.details,...d};const desc=document.querySelector('.detail-desc');if(desc&&d.description)desc.textContent=cleanDesc(d.description);}}catch(e){}
  // Enrich with Jikan if mal_id available
  if(S.anime.mal_id&&S.view==='detail'){invoke('jikan_detail',{malId:S.anime.mal_id}).then(j=>{if(S.view!=='detail')return;if(j.synopsis){const desc=document.querySelector('.detail-desc');if(desc&&!desc.textContent)desc.textContent=cleanDesc(j.synopsis);}if(j.genres?.length){const g=document.querySelector('.detail-genres');if(g)g.innerHTML=j.genres.map(g=>`<span class="tag">${esc(g)}</span>`).join('');}}).catch(()=>{});}
}
function watch(){if(S.episodes.length)getLinks(S.episodes[0]);}

async function getLinks(ep){S.currentEp=ep;S.links=[];transitionTo('loading',async()=>{S.view='loading';render();try{S.links=await invoke('get_episode_url',{showId:S.anime.id,episode:ep,mode:S.mode,quality:'best',title:S.anime.name||''});clearInterval(progAnim);S.view='player';render();}catch(e){clearInterval(progAnim);const t=document.getElementById('ptxt');if(t)t.textContent='Error: '+e;}});}

// Transition wrapper - animates out then switches view
function transitionTo(newView,cb){
  const app=document.getElementById('app');
  gsap.to(app,{opacity:0,y:-8,duration:.15,ease:'power2.in',onComplete:()=>{
    if(cb)cb();
    else{S.view=newView;render();}
  }});
}

// Navigation
function goHome(){
  // Find home in history and go back to it
  for(let i=navIndex-1;i>=0;i--){if(navHistory[i]?.view==='home'){navIndex=i;navLock=true;Object.assign(S,navHistory[i]);render();navLock=false;return;}}
  S.view='home';S.results=[];S.query='';render();pushNav();
}
function navToView(target){
  // Find the target view in history and go back to it
  for(let i=navIndex-1;i>=0;i--){
    if(navHistory[i]?.view===target){navIndex=i;navLock=true;Object.assign(S,navHistory[i]);render();navLock=false;return;}
  }
  // Fallback: just set view
  S.view=target;render();
}
async function doSearch(){S.query=document.getElementById('q')?.value?.trim()||S.query;if(!S.query)return;document.getElementById('aclist')?.classList.remove('show');S.results=[];S.view='search';render();await fetchResults();}
function setMode(mode,search){S.mode=mode;if(search)doSearch();else renderFilters(false);}
function setCountry(c,search){S.country=c;if(search)doSearch();else renderFilters(false);}

async function fetchResults(){
  const st=document.getElementById('status');if(st)st.textContent='Searching...';
  try{
    const items=await invoke('search_anime',{query:S.query,mode:S.mode,country:S.country});
    S.results=items;
    renderCards();
    if(st)st.textContent=S.results.length?'':'No results.';
  }catch(e){if(st)st.textContent='Error: '+e;}
}
window.addEventListener('scroll',()=>{if(navIndex>=0&&navHistory[navIndex])navHistory[navIndex].scrollY=window.scrollY;});

// Library
async function addToLibrary(tag){if(!tag||!S.anime)return;await invoke('save_history',{id:S.anime.id,name:S.anime.name||S.details?.name||'',episode:'1',mode:S.mode,progress:0});await invoke('set_history_tag',{id:S.anime.id,mode:S.mode,tag});if(S.details?.thumbnail)await invoke('set_history_thumbnail',{id:S.anime.id,mode:S.mode,thumbnail:S.details.thumbnail});}
async function showLibrary(){S.libTag='';transitionTo('library',()=>{S.view='library';render();});}
async function loadLibrary(){
  const items=S.libTag?await invoke('get_history_by_tag',{tag:S.libTag}):await invoke('get_history');
  const el=document.getElementById('lib-content');if(!el)return;
  if(!items.length){el.innerHTML='<div class="status-msg">Nothing here yet</div>';return;}
  const tags={watching:'eye',plan_to_watch:'clock',break:'pause',finished:'check-circle',dropped:'x-circle'};
  el.innerHTML=`<ul class="list">${items.map(h=>`<li class="li" ${A('resumeH2',[h.id,h.mode])}>
    <div style="display:flex;align-items:center;gap:8px">
      ${h.thumbnail?`<img src="${esc(h.thumbnail)}" style="width:36px;height:50px;object-fit:cover;border-radius:4px"/>`:''}
      <div><span class="t">${esc(h.name)}</span><br><span style="font-size:9px;color:var(--text2)">Ep ${esc(h.episode)} · ${fmt(h.progress)} <i data-lucide="${tags[h.tag]||'circle'}" style="width:10px;height:10px;display:inline-block;vertical-align:middle"></i></span></div>
    </div>
    <div style="display:flex;gap:4px;align-items:center">
      <select class="sinput" style="width:auto;padding:3px 6px;font-size:9px" data-nostop ${C('setTag',[h.id,h.mode])}>
        <option value="watching" ${h.tag==='watching'?'selected':''}>Watching</option>
        <option value="plan_to_watch" ${h.tag==='plan_to_watch'?'selected':''}>Plan to Watch</option>
        <option value="break" ${h.tag==='break'?'selected':''}>On Break</option>
        <option value="finished" ${h.tag==='finished'?'selected':''}>Finished</option>
        <option value="dropped" ${h.tag==='dropped'?'selected':''}>Dropped</option>
      </select>
      <span class="del" ${A('delH2',[h.id,h.mode])}>✕</span>
    </div>
  </li>`).join('')}</ul>`;
  if(window.lucide)try{lucide.createIcons();}catch(e){}
}
async function setTag(id,mode,tag){await invoke('set_history_tag',{id,mode,tag});loadLibrary();}
function setLibTag(tag){S.libTag=tag;updateLibFilters();loadLibrary();}
function updateLibFilters(){const btns=document.querySelectorAll('#lib-filters .btn');const labels=['watching','plan_to_watch','break','finished','dropped',''];btns.forEach((b,i)=>b.classList.toggle('active',labels[i]===S.libTag));}
async function delH2(id,mode){await invoke('delete_history',{id,mode});loadLibrary();}
async function resumeH2(id,mode){const items=await invoke('get_history');const h=items.find(e=>e.id===id&&e.mode===mode);if(!h)return;S.anime={id:h.id,name:h.name};S.mode=h.mode;S.resumeProgress=h.progress;const eps=await invoke('get_episodes',{showId:h.id,mode:h.mode});S.episodes=eps;S.view='detail';S.details={id:h.id,name:h.name,thumbnail:h.thumbnail||'',description:'',genres:[],status:'',episode_count:eps.length,rating:0};render();getLinks(h.episode);}

// Bookmarks
async function showBookmarks(){S.bookmarks=await invoke('get_bookmarks');transitionTo('bookmarks',()=>{S.view='bookmarks';render();});}
async function delBm(id){await invoke('delete_bookmark',{id});S.bookmarks=await invoke('get_bookmarks');render();}
function loadBm(i){const b=S.bookmarks[i];S.query=b.query;S.mode=b.mode;S.country=b.country;S.sort=b.sort;doSearch();}
function openSaveBookmark(){const q=document.getElementById('q')?.value||S.query;const o=document.createElement('div');o.className='modal-overlay';o.innerHTML=`<div class="modal"><h3>Save Bookmark</h3><input class="sinput" id="bmn" placeholder="Name..." value="${esc(q)} (${S.mode}/${S.country})"/><div class="modal-actions"><button class="btn bg" ${A('closeModal')}>Cancel</button><button class="btn bp" ${A('doSaveBm')}>Save</button></div></div>`;document.body.appendChild(o);}
function closeModal(){ctx.el?.closest('.modal-overlay')?.remove();}
async function doSaveBm(){const n=document.getElementById('bmn').value;await invoke('save_bookmark',{name:n,query:S.query,mode:S.mode,country:S.country,sort:S.sort});document.querySelector('.modal-overlay')?.remove();}

// Debug
async function debugSources(){const el=document.getElementById('status');if(el)el.innerHTML='Debugging...';try{const d=await invoke('debug_episode_url',{showId:S.anime.id,episode:S.currentEp,mode:S.mode});if(el)el.innerHTML=`<pre style="font-size:9px;color:var(--text2);background:var(--s1);padding:10px;border-radius:8px;overflow:auto;max-height:250px;text-align:left;white-space:pre-wrap">${esc(`GET: ${d.get_url}\nLen: ${d.get_response_len}\nSnippet: ${d.get_response_snippet?.slice(0,200)}\nPOST fallback: ${d.used_post_fallback}\ntobeparsed: ${d.has_tobeparsed}\nSources: ${d.sources_count}\n${JSON.stringify(d.sources,null,1)}\nProviders: ${JSON.stringify(d.decoded_provider_ids,null,1)}\nLinks: ${JSON.stringify(d.links_per_provider,null,1)}`)}</pre>`;}catch(e){if(el)el.textContent='Debug error: '+e;}}

// Downloads
let bulkMode=false,selectedEps=new Set();
function toggleBulk(){bulkMode=!bulkMode;selectedEps.clear();const b=document.getElementById('bulkbtn'),d=document.getElementById('dlselbtn');if(b)b.textContent=bulkMode?'Cancel':'Select';if(d)d.style.display=bulkMode?'inline-flex':'none';document.querySelectorAll('.ep-btn').forEach(b=>b.classList.remove('active'));}
function epClick(ep){const el=ctx.el;if(!bulkMode){getLinks(ep);return;}if(selectedEps.has(ep)){selectedEps.delete(ep);el.classList.remove('active');}else{selectedEps.add(ep);el.classList.add('active');}const d=document.getElementById('dlselbtn');if(d)d.textContent=`↓ DL (${selectedEps.size})`;}
async function downloadSelected(){if(!selectedEps.size)return;await invoke('start_bulk_download',{animeId:S.anime.id,animeName:S.anime.name||S.details?.name||'',episodes:[...selectedEps],mode:S.mode});toggleBulk();showDownloads();}
async function showDownloads(){transitionTo('downloads',()=>{S.view='downloads';render();});}
async function playOffline(path){
  if(!path){alert('No file path');return;}
  try{
    const url=await invoke('local_url',{path});
    S.anime=S.anime||{id:'offline',name:'Downloaded'};
    S.view='player';S.links=[{quality:'offline',url}];S.currentEp='offline';S.episodes=[];render();
  }catch(e){alert('Error: '+e);}
}
async function cancelDownload(id){await invoke('cancel_download',{id});}
async function removeDownload(id){await invoke('remove_download',{id});refreshDownloads();}
function openDownloadFolder(){invoke('open_download_folder');}
async function refreshDownloads(){const items=await invoke('get_downloads');const el=document.getElementById('dl-list');if(!el)return;if(!items.length){el.innerHTML='<div class="status-msg">No downloads</div>';return;}
  const groups={};items.forEach(d=>{if(!groups[d.anime_name])groups[d.anime_name]=[];groups[d.anime_name].push(d);});
  el.innerHTML=Object.entries(groups).map(([name,eps])=>`
    <div style="margin-bottom:12px">
      <div class="slabel">${esc(name)}</div>
      <ul class="list">${eps.map(d=>`<li class="li" style="padding:8px 12px;flex-direction:column;align-items:stretch;gap:4px">
        <div style="display:flex;justify-content:space-between;align-items:center">
          <span class="t">Ep ${esc(d.episode)}</span>
          <div style="display:flex;gap:4px;align-items:center">
            ${d.status==='Done'?`<button class="btn bp" style="padding:2px 8px;font-size:9px" ${A('playOffline',[d.file_path])}>▶</button>`:''}
            ${d.status==='Downloading'?`<button class="btn bg" style="padding:2px 6px;font-size:9px" ${A('cancelDownload',[d.id])}>Cancel</button>`:''}
            ${d.status==='Done'||d.status==='Failed'?`<button class="btn bg" style="padding:2px 6px;font-size:9px" ${A('removeDownload',[d.id])}>✕</button>`:''}
            <span class="m">${esc(d.status)}</span>
          </div>
        </div>
        ${d.status==='Downloading'||d.status==='Queued'?`
          <div style="display:flex;align-items:center;gap:8px">
            <div style="flex:1;height:3px;background:var(--border);border-radius:3px;overflow:hidden"><div style="height:100%;width:${Number(d.progress)||0}%;background:linear-gradient(90deg,var(--accent2),var(--accent));transition:width .3s"></div></div>
            <span style="font-size:9px;color:var(--text2);white-space:nowrap">${Math.round(d.progress)}% · ${fmtBytes(d.downloaded_bytes)}/${fmtBytes(d.total_bytes)}</span>
          </div>`:''}
        ${d.error?`<span style="font-size:9px;color:var(--accent2)">${esc(d.error)}</span>`:''}
      </li>`).join('')}</ul>
    </div>`).join('');
  if(items.some(d=>d.status==='Downloading'||d.status==='Queued')){clearTimeout(dlRef);dlRef=setTimeout(refreshDownloads,1500);}}

// Player
function prevEp(){const i=S.episodes.indexOf(S.currentEp);if(i>0)getLinks(S.episodes[i-1]);}
function nextEp(){const i=S.episodes.indexOf(S.currentEp);if(i<S.episodes.length-1)getLinks(S.episodes[i+1]);}
function animProg(){let p=0;clearInterval(progAnim);progAnim=setInterval(()=>{p=Math.min(p+Math.random()*6,92);const b=document.getElementById('pbar');if(b)b.style.width=p+'%';},250);}

async function playSrc(i){
  const v=document.getElementById('vp');if(!v)return;
  document.querySelectorAll('.qbtn').forEach((b,idx)=>{b.classList.toggle('bp',idx===i);b.classList.toggle('bg',idx!==i);});
  if(window._hls){window._hls.destroy();window._hls=null;}
  let url=S.links[i].url;
  if(!url.includes('.m3u8')&&!url.startsWith('http://127.0.0.1')){try{url=await invoke('proxy_url',{url,referrer:S.links[i].referrer||'https://allmanga.to'});}catch(e){}}
  if(url.includes('.m3u8')){
    if(Hls.isSupported()){const hls=new Hls();hls.loadSource(url);hls.attachMedia(v);hls.on(Hls.Events.MANIFEST_PARSED,()=>{seekR(v);v.play();});hls.on(Hls.Events.ERROR,(_,d)=>{if(d.fatal)document.getElementById('status').textContent='Playback error';});window._hls=hls;}
    else if(v.canPlayType('application/vnd.apple.mpegurl')){v.src=url;v.onloadedmetadata=()=>seekR(v);}
  }else{v.src=url;v.onloadedmetadata=()=>seekR(v);v.play();}
  clearInterval(saveInt);saveInt=setInterval(()=>saveP(v),5000);
  v.onpause=()=>{saveP(v);updPlay();document.getElementById('pwrap')?.classList.add('paused');};
  v.onplay=()=>{updPlay();document.getElementById('pwrap')?.classList.remove('paused');};
  v.onended=()=>{saveP(v);if(S.autoNext)nextEp();};
  v.ontimeupdate=()=>{updSeek(v);checkSkip(v);};v.onprogress=()=>updBuf(v);
  // Load skip times
  loadSkipTimes();
}

// Player controls
function togglePlay(){const v=document.getElementById('vp');if(!v)return;v.paused?v.play():v.pause();}
function updPlay(){const v=document.getElementById('vp'),ico=document.getElementById('playico');if(!v||!ico)return;ico.setAttribute('d',v.paused?'M8 5v14l11-7z':'M6 19h4V5H6v14zm8-14v14h4V5h-4z');}
function skip(s){const v=document.getElementById('vp');if(v)v.currentTime=Math.max(0,Math.min(v.duration||0,v.currentTime+s));}
function seek(e){const v=document.getElementById('vp'),bar=document.getElementById('seekbar');if(!v||!bar||!v.duration)return;const r=bar.getBoundingClientRect();v.currentTime=(e.clientX-r.left)/r.width*v.duration;}
function updSeek(v){const p=document.getElementById('seekprog'),t=document.getElementById('ctime');if(!p||!v.duration)return;p.style.width=(v.currentTime/v.duration*100)+'%';t.textContent=fmtT(v.currentTime)+' / '+fmtT(v.duration);}
function updBuf(v){const b=document.getElementById('seekbuf');if(!b||!v.buffered.length||!v.duration)return;b.style.width=(v.buffered.end(v.buffered.length-1)/v.duration*100)+'%';}
function fmtT(s){const h=Math.floor(s/3600),m=Math.floor(s%3600/60),sec=Math.floor(s%60);return(h?h+':':'')+(h&&m<10?'0':'')+m+':'+(sec<10?'0':'')+sec;}
function setVol(val){const v=document.getElementById('vp');if(v){v.volume=val;v.muted=false;}}
function toggleMute(){const v=document.getElementById('vp');if(v)v.muted=!v.muted;}
const speeds=[.5,.75,1,1.25,1.5,2];let spdIdx=2;
function cycleSpeed(){spdIdx=(spdIdx+1)%speeds.length;const v=document.getElementById('vp'),b=document.getElementById('speedbtn');if(v)v.playbackRate=speeds[spdIdx];if(b)b.textContent=speeds[spdIdx]+'x';}
function toggleFS(){const w=document.getElementById('pwrap');if(!w)return;document.fullscreenElement?document.exitFullscreen():w.requestFullscreen();}
function seekR(v){if(S.resumeProgress&&S.resumeProgress>5){v.currentTime=S.resumeProgress;S.resumeProgress=null;}}

// AniSkip
let skipTimes=[],currentSkip=null;
async function loadSkipTimes(){
  skipTimes=[];currentSkip=null;
  const malId=S.anime?.mal_id;const ep=parseInt(S.currentEp);
  if(!malId||!ep)return;
  try{skipTimes=await invoke('get_skip_times',{malId,episode:ep,epLength:null});}catch(e){}
}
function checkSkip(v){
  const t=v.currentTime;const btn=document.getElementById('skipbtn');if(!btn)return;
  const active=skipTimes.find(s=>t>=s.start&&t<s.end-1);
  if(active){
    currentSkip=active;
    const label=active.skip_type==='op'?'Skip Opening ▶':active.skip_type==='ed'?'Skip Ending ▶':'Skip ▶';
    btn.textContent=label;btn.classList.remove('hidden');
  }else{btn.classList.add('hidden');currentSkip=null;}
}
function doSkip(){if(currentSkip){const v=document.getElementById('vp');if(v)v.currentTime=currentSkip.end;}}
async function saveP(v){if(!v||!S.anime||!S.currentEp)return;const p=v.currentTime||0;if(p<2)return;await invoke('save_history',{id:S.anime.id,name:S.anime.name,episode:S.currentEp,mode:S.mode,progress:p}).catch(()=>{});if(S.anime.thumbnail||S.details?.thumbnail)invoke('set_history_thumbnail',{id:S.anime.id,mode:S.mode,thumbnail:S.anime.thumbnail||S.details?.thumbnail||''}).catch(()=>{});}

// Keyboard shortcuts (YouTube-like)
document.addEventListener('keydown',e=>{
  // Back/forward navigation
  if(e.altKey&&e.key==='ArrowLeft'){e.preventDefault();navBack();return;}
  if(e.altKey&&e.key==='ArrowRight'){e.preventDefault();navForward();return;}
  if(S.view!=='player'||e.target.tagName==='INPUT'||e.target.tagName==='SELECT'||e.target.tagName==='TEXTAREA')return;
  const v=document.getElementById('vp');if(!v)return;
  switch(e.key){
    case ' ':case 'k':case 'K':e.preventDefault();togglePlay();break;
    case 'ArrowLeft':case 'j':case 'J':e.preventDefault();skip(-5);break;
    case 'ArrowRight':case 'l':case 'L':e.preventDefault();skip(5);break;
    case 'ArrowUp':e.preventDefault();v.volume=Math.min(1,v.volume+.1);break;
    case 'ArrowDown':e.preventDefault();v.volume=Math.max(0,v.volume-.1);break;
    case 'f':case 'F':e.preventDefault();toggleFS();break;
    case 'm':case 'M':e.preventDefault();toggleMute();break;
    case 'N':if(e.shiftKey){e.preventDefault();nextEp();}break;
    case 'P':if(e.shiftKey){e.preventDefault();prevEp();}break;
    case '>':e.preventDefault();cycleSpeed();break;
  }
});

// Breadcrumbs: each crumb is a label plus an optional action
function bc(){
  const crumbs=[{label:'Home',act:'goHome'}];
  for(let i=1;i<=navIndex;i++){
    const s=navHistory[i];if(!s)continue;
    if(s.view==='search'&&!crumbs.find(c=>c.label==='Search'))crumbs.push({label:'Search',act:'navToView',args:['search']});
    if(s.view==='detail'&&s.anime)crumbs.push({label:s.anime.name||'Detail',act:'navToView',args:['detail']});
    if(s.view==='player'&&s.currentEp)crumbs.push({label:'Ep '+s.currentEp});
    if(s.view==='library')crumbs.push({label:'Library'});
    if(s.view==='bookmarks')crumbs.push({label:'Bookmarks'});
    if(s.view==='downloads')crumbs.push({label:'Downloads'});
  }
  // Add current if not already there
  if(S.view==='detail'&&S.anime&&!crumbs.find(c=>c.label===S.anime.name))crumbs.push({label:S.anime.name});
  if(S.view==='player'&&S.currentEp&&!crumbs.find(c=>c.label==='Ep '+S.currentEp))crumbs.push({label:'Ep '+S.currentEp});
  if(S.view==='library'&&!crumbs.find(c=>c.label==='Library'))crumbs.push({label:'Library'});
  if(S.view==='bookmarks'&&!crumbs.find(c=>c.label==='Bookmarks'))crumbs.push({label:'Bookmarks'});
  if(S.view==='downloads'&&!crumbs.find(c=>c.label==='Downloads'))crumbs.push({label:'Downloads'});
  return`<div class="breadcrumbs">${crumbs.map((c,i)=>i<crumbs.length-1?`<span ${c.act?A(c.act,c.args||[]):''}>${esc(c.label)}</span><span class="sep">›</span>`:`<span class="current">${esc(c.label)}</span>`).join('')}</div>`;
}

function animate(){
  const app=document.getElementById('app');
  if(window.lucide)try{lucide.createIcons();}catch(e){}
  if(!window.gsap){app.style.opacity='1';return;}
  gsap.fromTo(app,{opacity:0,y:8},{opacity:1,y:0,duration:.3,ease:'power2.out'});
  gsap.from('.card',{y:20,opacity:0,duration:.35,stagger:.04,ease:'power2.out',delay:.15});
  gsap.from('.li',{x:-15,opacity:0,duration:.3,stagger:.05,ease:'power2.out',delay:.1});
  gsap.from('.detail',{y:15,opacity:0,duration:.4,ease:'power2.out',delay:.1});
  gsap.from('.player-wrap',{scale:.97,opacity:0,duration:.4,ease:'power2.out',delay:.1});
  gsap.from('.player-sidebar',{x:20,opacity:0,duration:.35,ease:'power2.out',delay:.2});
  gsap.from('.epgrid .btn',{scale:.8,opacity:0,duration:.25,stagger:.02,ease:'back.out(1.5)',delay:.15});
  gsap.from('.filters',{y:-10,opacity:0,duration:.3,ease:'power2.out'});
  gsap.from('.loader',{opacity:0,duration:.3,ease:'power2.out'});
  gsap.from('.pbar',{y:10,opacity:0,duration:.3,ease:'power2.out',delay:.25});
  gsap.from('.qrow',{y:10,opacity:0,duration:.3,ease:'power2.out',delay:.3});
}

// Every action named in markup. Each receives the JSON args from data-args.
const ACTIONS={
  goHome:()=>goHome(),
  navToView:t=>navToView(t),
  showLibrary:()=>showLibrary(),
  showDownloads:()=>showDownloads(),
  showBookmarks:()=>showBookmarks(),
  doSearch:()=>doSearch(),
  setMode:(m,search)=>setMode(m,search),
  setCountry:(c,search)=>setCountry(c,search),
  openSaveBookmark:()=>openSaveBookmark(),
  closeModal:()=>closeModal(),
  doSaveBm:()=>doSaveBm(),
  openDetail:i=>openDetail(i),
  openHomeCard:()=>openHomeCard(),
  acSelect:i=>acSelect(i),
  expandSection:id=>expandSection(id),
  setViewMode:m=>setViewMode(m),
  watch:()=>watch(),
  getLinks:ep=>getLinks(ep),
  epClick:ep=>epClick(ep),
  toggleBulk:()=>toggleBulk(),
  downloadSelected:()=>downloadSelected(),
  addToLibrary:()=>addToLibrary(ctx.el.value),
  setAutoNext:()=>{S.autoNext=ctx.el.checked;},
  setVol:()=>setVol(ctx.el.value),
  togglePlay:()=>togglePlay(),
  doSkip:()=>doSkip(),
  seek:()=>seek(ctx.ev),
  skip:s=>skip(s),
  toggleMute:()=>toggleMute(),
  cycleSpeed:()=>cycleSpeed(),
  toggleFS:()=>toggleFS(),
  prevEp:()=>prevEp(),
  nextEp:()=>nextEp(),
  playSrc:i=>playSrc(i),
  debugSources:()=>debugSources(),
  setLibTag:t=>setLibTag(t),
  setTag:(id,mode)=>setTag(id,mode,ctx.el.value),
  resumeH2:(id,mode)=>resumeH2(id,mode),
  delH2:(id,mode)=>delH2(id,mode),
  loadBm:i=>loadBm(i),
  delBm:id=>delBm(id),
  playOffline:path=>playOffline(path),
  cancelDownload:id=>cancelDownload(id),
  removeDownload:id=>removeDownload(id),
  openDownloadFolder:()=>openDownloadFolder(),
};

render();pushNav();
// Mouse back/forward buttons
window.addEventListener('mouseup',e=>{if(e.button===3)navBack();if(e.button===4)navForward();});
