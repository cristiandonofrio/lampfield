// Pure board logic, no DOM, so it can be tested with `node --test`.

export const STATES = {
  need: { label: 'Needs you', icon: '?', word: 'need you' },
  quiet: { label: 'May need you', icon: '!', word: 'may need you' },
  fail: { label: 'Failed', icon: '×', word: 'failed' },
  done: { label: 'Finished', icon: '✓', word: 'finished' },
  work: { label: 'Working', icon: '', word: 'working' },
  idle: { label: 'Idle', icon: '', word: 'idle' },
};

/** The tile's look. `seen` maps session id to the turn end you marked as seen. */
export function tileState(s, seen = {}) {
  if (s.state === 'needs') return 'need';
  if (s.state === 'failed') return 'fail';
  if (s.state === 'working') return s.label === 'Quiet' ? 'quiet' : 'work';
  if (s.unread && !((seen[s.id] ?? 0) >= s.turnEnd)) return 'done';
  return 'idle';
}

/** The session you last messaged comes first. A tile only moves when you start a session
 *  or reply to one, never because it finished or asked something. */
export function ordered(sessions) {
  const key = (s) => s.lastUsed || s.createdAt || 0;
  return [...sessions].sort((a, b) => (key(b) - key(a)) || a.id.localeCompare(b.id));
}

export function tileLine(s, st) {
  if (st === 'need' && s.question) return s.question.text;
  if (st === 'idle' && s.label === 'Stopped') return 'Stopped partway through';
  if (st === 'idle' && s.label === 'Dismissed') return 'Dismissed';
  const helpers = st === 'work' && s.helpers ? ` · ${s.helpers} helper${s.helpers > 1 ? 's' : ''}` : '';
  return (s.doing || STATES[st].label) + helpers;
}

export function counts(snapshot, seen) {
  const n = { need: 0, quiet: 0, fail: 0, done: 0, work: 0, idle: 0 };
  for (const g of snapshot?.groups ?? []) for (const s of g.sessions) n[tileState(s, seen)]++;
  return n;
}

/** Sessions that have just started needing you or failed, for the chime. */
export function newAlerts(before, after, seen) {
  if (!before) return [];
  const old = new Map(before.groups.flatMap((g) => g.sessions).map((s) => [s.id, tileState(s, seen)]));
  return after.groups
    .flatMap((g) => g.sessions)
    .filter((s) => {
      const st = tileState(s, seen);
      return (st === 'need' || st === 'fail') && old.get(s.id) !== st;
    })
    .map((s) => s.id);
}

/** Split groups into what to draw and what's hidden. Groups emptied by hiding are dropped,
 *  groups that were empty anyway stay, so they can be listed as empty. */
export function splitHidden(groups, ids) {
  const set = new Set(ids);
  const hidden = groups.flatMap((group) => group.sessions.filter((s) => set.has(s.id)).map((session) => ({ group, session })));
  const visible = groups
    .map((g) => ({ ...g, sessions: g.sessions.filter((s) => !set.has(s.id)), wasEmpty: !g.sessions.length }))
    .filter((g) => g.sessions.length || g.wasEmpty);
  return { visible, hidden };
}

export function ago(ms, now = Date.now()) {
  if (!ms) return '';
  const m = Math.max(0, Math.floor((now - ms) / 60000));
  if (m < 1) return 'now';
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

export function baseName(p = '') {
  const t = p.replace(/[\\/]+$/, '');
  return t.split(/[\\/]/).pop() || t;
}
