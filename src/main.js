import './style.css';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { STATES, tileState, ordered, tileLine, counts, newAlerts, ago, baseName, splitHidden } from './board.js';
import { demoSnapshot } from './demo.js';

const inApp = !!window.__TAURI_INTERNALS__;
const $ = (id) => document.getElementById(id);
const board = $('board'), drop = $('drop'), toast = $('toast'), hiddenPanel = $('hiddenpanel');

const store = {
  get: (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } },
  set: (k, v) => localStorage.setItem(k, JSON.stringify(v)),
};
let seen = store.get('lampfield.seen', {});
let muted = store.get('lampfield.muted', false);
let onTop = store.get('lampfield.onTop', false);
let hidden = store.get('lampfield.hidden', []);
let hiddenOpen = false;
let snap = null, openId = null, tabFor = {}, filter = null;
const chime = new Audio('/chime.wav');

const esc = (t) => String(t ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const all = () => (snap?.groups ?? []).flatMap((g) => g.sessions);
const find = (id) => all().find((s) => s.id === id);
const groupOf = (id) => snap.groups.find((g) => g.sessions.some((s) => s.id === id));

function say(m) { toast.textContent = m; toast.classList.add('on'); clearTimeout(say.t); say.t = setTimeout(() => toast.classList.remove('on'), 1600); }

function split() { return splitHidden(snap.groups, hidden); }

function renderHeader() {
  const { visible, hidden: hid } = split();
  const n = counts({ groups: visible }, seen);
  const hb = $('hiddenbtn');
  hb.hidden = !hid.length;
  hb.textContent = `Hidden ${hid.length}`;
  // A hidden session that needs you still shows, so it can't be missed.
  hb.classList.toggle('alert', hid.some(({ session }) => ['need', 'fail'].includes(tileState(session, seen))));
  const order = ['need', 'quiet', 'fail', 'done', 'work'];
  $('counts').innerHTML = order.filter((k) => n[k]).map((k) =>
    `<button class="chip s-${k}" data-k="${k}" aria-pressed="${filter === k}"><span class="ic">${STATES[k].icon}</span>${n[k]} <span class="k">${STATES[k].word}</span></button>`).join('')
    || '<span class="note" style="margin:0">Nothing needs you</span>';
  $('counts').querySelectorAll('.chip').forEach((c) => c.onclick = (e) => { e.stopPropagation(); filter = filter === c.dataset.k ? null : c.dataset.k; render(); });
  $('ontop').setAttribute('aria-pressed', onTop);
  $('sound').setAttribute('aria-pressed', !muted);
  $('sound').textContent = muted ? 'Sound off' : 'Sound on';
  const note = snap.groupSource === 'folder' && snap.groupNote ? `Grouped by project folder, as the sidebar groups couldn't be read (${snap.groupNote}).` : '';
  $('note').textContent = note;
  $('note').hidden = !note;
}

function tileHTML(s) {
  const st = tileState(s, seen);
  const dim = filter && filter !== st ? ' dimmed' : '';
  return `<button class="tile s-${st}${openId === s.id ? ' open' : ''}${dim}" data-id="${esc(s.id)}" title="${esc(s.title)}&#10;Click to open in Claude. Right-click for details." aria-label="${esc(s.title)}, ${STATES[st].label}">
    <div class="top"><span class="ic">${STATES[st].icon}</span><span class="name">${esc(s.title)}</span></div>
    <div class="line">${esc(tileLine(s, st))}</div><div class="age">${st === 'done' ? 'unseen' : ago(s.lastActivity)}</div></button>`;
}

function render() {
  if (!snap) return;
  renderHeader();
  const tiles = all().length;
  const { visible } = split();
  if (!tiles) {
    board.innerHTML = `<div class="empty"><p>No Claude Code sessions found.</p><p>Looking in <code>${esc(snap.sessionsRoot)}</code>. Start a session in the Claude desktop app's Code tab and it will appear here.</p></div>`;
  } else if (!visible.some((g) => g.sessions.length)) {
    board.innerHTML = '<div class="empty"><p>Every session is hidden. Use Hidden at the top to show them again.</p></div>';
  } else {
    // Empty groups share one line at the end, so the tiles stay together.
    const full = visible.filter((g) => g.sessions.length);
    const empty = visible.filter((g) => !g.sessions.length);
    board.innerHTML = full.map((g) => `
      <section class="group"><div class="ghead"><b>${esc(g.name)}</b><span>${esc(baseName(g.folder))}</span></div>
      <div class="grid">${ordered(g.sessions).map(tileHTML).join('')}</div></section>`).join('')
      + (empty.length ? `<p class="emptygroups">No sessions in ${empty.map((g) => esc(g.name)).join(', ')}</p>` : '');
    // A click opens the session in Claude, a right click opens its panel.
    board.querySelectorAll('.tile').forEach((t) => {
      t.onclick = (e) => { e.stopPropagation(); if (openId) close(true); const s = find(t.dataset.id); if (s) act(s, 'open'); };
      t.oncontextmenu = (e) => { e.preventDefault(); e.stopPropagation(); toggle(t.dataset.id); };
      t.onkeydown = (e) => { if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) { e.preventDefault(); toggle(t.dataset.id); } };
    });
  }
  if (openId && !find(openId)) close(true);
  if (openId) { renderDrop(); place(); }
  renderHidden();
  if (inApp) {
    const states = Object.entries(counts(snap, seen)).filter(([, v]) => v).map(([k, v]) => `${k}=${v}`).join(' ');
    invoke('report_rendered', { groups: snap.groups.length, tiles, states }).catch(() => {});
  }
}

function toggle(id) {
  if (openId === id) return close();
  openId = id; tabFor[id] ??= 'now';
  drop.classList.add('on'); render();
}
function close(quiet) { openId = null; drop.classList.remove('on'); if (!quiet) render(); }
function place() {
  const t = board.querySelector(`.tile[data-id="${CSS.escape(openId)}"]`); if (!t) return;
  const r = t.getBoundingClientRect(), w = Math.min(370, window.innerWidth - 20);
  let left = r.left + window.scrollX;
  if (left + w > window.scrollX + window.innerWidth - 10) left = window.scrollX + window.innerWidth - 10 - w;
  drop.style.left = Math.max(10, left) + 'px';
  drop.style.top = r.bottom + window.scrollY + 6 + 'px';
}

function nowPane(s, st) {
  if (st === 'need' && s.question) {
    return `<p class="q">${esc(s.question.text)}</p>
      ${s.question.options.length ? `<ol class="opts">${s.question.options.map((o, i) => `<li><kbd>${i + 1}</kbd>${esc(o)}</li>`).join('')}</ol>` : ''}
      <p class="dim">Answer it in Claude.</p>`;
  }
  if (st === 'need') return `<p class="q">${esc(s.doing || 'Waiting for you')}</p><p class="dim">Reply in Claude.</p>`;
  if (st === 'quiet') return `<p class="q">It may be waiting for you to approve something.</p><p class="dim">${esc(s.doing)}</p>`;
  if (st === 'fail') return `<p class="q">It stopped with an error.</p><p class="dim">${esc(s.doing)}</p>`;
  if (st === 'done') return `<p class="q">Finished.</p><p class="dim">${esc(s.doing)}</p>`;
  if (st === 'work') return `<p class="q">Working.</p><p class="dim">${esc(tileLine(s, st))}. Nothing needed from you.</p>`;
  return `<p class="dim">${esc(tileLine(s, st))}</p>`;
}

function renderDrop() {
  const s = find(openId); if (!s) return;
  const st = tileState(s, seen), tab = tabFor[s.id];
  let body;
  if (tab === 'changes') {
    const add = s.files.reduce((n, f) => n + f.added, 0), rem = s.files.reduce((n, f) => n + f.removed, 0);
    body = s.files.length
      ? `<p class="dim">${s.files.length} file${s.files.length > 1 ? 's' : ''}, <span class="a">+${add}</span> <span class="d">-${rem}</span></p><ul class="files">${s.files.map((f) => `<li title="${esc(f.path)}"><span>${esc(f.name)}</span><span><span class="a">+${f.added}</span> <span class="d">-${f.removed}</span></span></li>`).join('')}</ul>`
      : '<p class="dim">No file changes in this session.</p>';
  } else if (tab === 'steps') {
    body = s.steps.length ? `<ul class="steps">${s.steps.map((x) => `<li><span>${esc(x.doing)}</span><span>${ago(x.at)}</span></li>`).join('')}</ul>` : '<p class="dim">No steps yet.</p>';
  } else body = nowPane(s, st);
  const tabs = [['now', 'Now'], ['changes', `Changes${s.files.length ? ' ' + s.files.length : ''}`], ['steps', 'Steps']];
  const acts = [];
  if (st === 'need' || st === 'fail') acts.push('<button class="btn ghost" data-act="dismiss">Dismiss</button>');
  if (st === 'done') acts.push('<button class="btn ghost" data-act="seen">Dismiss</button>');
  acts.push('<button class="btn ghost" data-act="hide" title="Take it off the board. Find it again under Hidden at the top.">Hide</button>');
  acts.push('<button class="btn" data-act="open">Open in Claude</button>');
  drop.innerHTML = `<div class="dh s-${st}"><span class="ic">${STATES[st].icon}</span><span class="name">${esc(s.title)}</span><span class="grp">${esc(groupOf(s.id)?.name)}</span><button class="x" aria-label="Close">×</button></div>
    <div class="tabs" role="tablist">${tabs.map(([k, l]) => `<button class="tab" role="tab" data-t="${k}" aria-selected="${k === tab}">${l}</button>`).join('')}</div>
    <div class="pane">${body}</div>
    <div class="foot"><span class="meta">${STATES[st].label} · ${esc(baseName(s.folder))}</span><span class="acts">${acts.join('')}</span></div>`;
  drop.querySelector('.x').onclick = () => close();
  drop.querySelectorAll('.tab').forEach((b) => b.onclick = () => { tabFor[s.id] = b.dataset.t; renderDrop(); });
  drop.querySelectorAll('[data-act]').forEach((b) => b.onclick = () => act(s, b.dataset.act));
}

function renderHidden() {
  const { hidden: hid } = split();
  if (!hid.length) hiddenOpen = false;
  hiddenPanel.classList.toggle('on', hiddenOpen);
  if (!hiddenOpen) return;
  hiddenPanel.innerHTML = `<div class="hh"><b>Hidden sessions</b><button class="btn" id="archiveall" title="Opens a new Claude session that archives these for you">Archive in Claude</button></div>
    <ul class="hlist">${hid.map(({ group, session }) => {
      const st = tileState(session, seen);
      return `<li class="s-${st}"><span class="ic">${STATES[st].icon}</span><span class="ht"><span class="name">${esc(session.title)}</span><span class="grp">${esc(group.name)}</span></span><button class="btn ghost" data-show="${esc(session.id)}">Show</button></li>`;
    }).join('')}</ul>`;
  const r = $('hiddenbtn').getBoundingClientRect(), w = Math.min(380, window.innerWidth - 20);
  hiddenPanel.style.top = r.bottom + 6 + 'px';
  hiddenPanel.style.left = Math.max(10, Math.min(r.left, window.innerWidth - 10 - w)) + 'px';
  hiddenPanel.querySelectorAll('[data-show]').forEach((b) => b.onclick = () => {
    hidden = hidden.filter((id) => id !== b.dataset.show); store.set('lampfield.hidden', hidden); render();
  });
  $('archiveall').onclick = async () => {
    const ids = hid.map(({ session }) => session.id);
    if (!inApp) { say(`Would ask Claude to archive ${ids.length} session${ids.length === 1 ? "" : "s"}`); return; }
    try { await invoke('archive_in_claude', { ids }); hiddenOpen = false; render(); } catch (e) { say(`Couldn't open Claude: ${e}`); }
  };
}

function markSeen(s) { seen[s.id] = s.turnEnd; store.set('lampfield.seen', seen); }

async function act(s, what) {
  if (what === 'open') {
    if (tileState(s, seen) === 'done') markSeen(s);
    if (!inApp) { say(`Would open "${s.title}" in Claude`); return; }
    try { await invoke('open_session', { id: s.id }); close(); } catch (e) { say(`Couldn't open it: ${e}`); }
  } else if (what === 'hide') {
    hidden = [...new Set([...hidden, s.id])]; store.set('lampfield.hidden', hidden); close(); say('Hidden. Find it under Hidden at the top.');
  } else if (what === 'seen') {
    markSeen(s); close(); say('Dismissed');
  } else if (what === 'dismiss') {
    if (!inApp) { say('Would dismiss it'); return; }
    try { await invoke('dismiss', { id: s.id }); close(); say('Dismissed until something new happens'); } catch (e) { say(`Couldn't dismiss: ${e}`); }
  }
}

function update(next) {
  const fresh = newAlerts(snap, next, seen);
  snap = next;
  render();
  for (const id of fresh) board.querySelector(`.tile[data-id="${CSS.escape(id)}"]`)?.classList.add('flash');
  if (fresh.length && !muted) chime.play().catch(() => {});
}

// The panel redraws itself on clicks, so the clicked button may be gone by the time the
// click reaches the page. Stopping it here keeps a click inside from reading as outside.
drop.addEventListener('click', (e) => e.stopPropagation());
drop.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); });
hiddenPanel.addEventListener('click', (e) => e.stopPropagation());
hiddenPanel.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); });
document.addEventListener('click', () => {
  if (openId) close();
  if (hiddenOpen) { hiddenOpen = false; renderHidden(); }
});
// No browser menu on a right click anywhere else, it would only offer Reload.
document.addEventListener('contextmenu', (e) => { e.preventDefault(); if (openId) close(); });
document.addEventListener('keydown', (e) => {
  if (!openId) return;
  if (e.key === 'Escape') close();
  if (e.key === 'Enter' && !e.target.closest('button')) { const s = find(openId); if (s) act(s, 'open'); }
});
window.addEventListener('resize', () => { if (openId) place(); if (hiddenOpen) renderHidden(); });
$('hiddenbtn').onclick = (e) => { e.stopPropagation(); if (openId) close(true); hiddenOpen = !hiddenOpen; renderHidden(); };
$('ontop').onclick = async (e) => {
  e.stopPropagation(); onTop = !onTop; store.set('lampfield.onTop', onTop); renderHeader();
  if (inApp) invoke('set_on_top', { on: onTop }).catch(() => {});
};
$('sound').onclick = (e) => {
  e.stopPropagation(); muted = !muted; store.set('lampfield.muted', muted); renderHeader();
  if (!muted) chime.play().catch(() => {});
};
// Ages on tiles move with time even when nothing else changes.
setInterval(() => { if (!openId) render(); }, 30_000);

if (inApp) {
  if (onTop) invoke('set_on_top', { on: true }).catch(() => {});
  listen('snapshot', (e) => update(e.payload));
  invoke('get_snapshot').then((s) => s && update(s));
} else {
  update(demoSnapshot());
}
