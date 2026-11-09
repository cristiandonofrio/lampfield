import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tileState, ordered, tileLine, counts, newAlerts, ago, baseName, splitHidden } from './board.js';

const s = (over) => ({ id: 'local_a', state: 'idle', label: 'Idle', doing: '', helpers: 0, createdAt: 1, turnEnd: 10, unread: false, question: null, ...over });

test('maps core states to tile looks', () => {
  assert.equal(tileState(s({ state: 'needs' })), 'need');
  assert.equal(tileState(s({ state: 'failed' })), 'fail');
  assert.equal(tileState(s({ state: 'working', label: 'Quiet' })), 'quiet');
  assert.equal(tileState(s({ state: 'working', label: 'Working' })), 'work');
  assert.equal(tileState(s({ unread: true })), 'done');
  assert.equal(tileState(s()), 'idle');
});

test('a finish marked as seen goes quiet until the next one', () => {
  assert.equal(tileState(s({ unread: true, turnEnd: 10 }), { local_a: 10 }), 'idle');
  assert.equal(tileState(s({ unread: true, turnEnd: 20 }), { local_a: 10 }), 'done');
});

test('the session you last messaged comes first, and finishing does not move it', () => {
  const list = [
    s({ id: 'local_old', createdAt: 1, lastUsed: 5 }),
    s({ id: 'local_new', createdAt: 9, lastUsed: 0 }),
    s({ id: 'local_replied', createdAt: 2, lastUsed: 20, turnEnd: 99 }),
  ];
  assert.deepEqual(ordered(list).map((x) => x.id), ['local_replied', 'local_new', 'local_old']);
});

test('tile lines say what matters', () => {
  assert.equal(tileLine(s({ question: { text: 'Which one?', options: [] } }), 'need'), 'Which one?');
  assert.equal(tileLine(s({ doing: 'Running tests', helpers: 2 }), 'work'), 'Running tests · 2 helpers');
  assert.equal(tileLine(s({ label: 'Stopped' }), 'idle'), 'Stopped partway through');
});

test('counts and new alerts', () => {
  const snap = (a, b) => ({ groups: [{ sessions: [s({ id: 'local_a', state: a }), s({ id: 'local_b', state: b })] }] });
  assert.equal(counts(snap('needs', 'working')).need, 1);
  assert.deepEqual(newAlerts(null, snap('needs', 'idle')), []);
  assert.deepEqual(newAlerts(snap('working', 'needs'), snap('needs', 'needs')), ['local_a']);
  assert.deepEqual(newAlerts(snap('needs', 'idle'), snap('needs', 'failed')), ['local_b']);
});

test('hidden sessions leave the board and groups they emptied go too', () => {
  const groups = [
    { id: 'g1', sessions: [s({ id: 'local_a' }), s({ id: 'local_b' })] },
    { id: 'g2', sessions: [s({ id: 'local_c' })] },
    { id: 'g3', sessions: [] },
  ];
  const { visible, hidden } = splitHidden(groups, ['local_b', 'local_c', 'local_gone']);
  assert.deepEqual(visible.map((g) => [g.id, g.sessions.map((x) => x.id)]), [['g1', ['local_a']], ['g3', []]]);
  assert.deepEqual(hidden.map((h) => [h.group.id, h.session.id]), [['g1', 'local_b'], ['g2', 'local_c']]);
});

test('ago and base names', () => {
  const now = 1_000_000_000;
  assert.equal(ago(now - 30_000, now), 'now');
  assert.equal(ago(now - 5 * 60_000, now), '5m');
  assert.equal(ago(now - 3 * 3_600_000, now), '3h');
  assert.equal(ago(0, now), '');
  assert.equal(baseName('C:\\code\\portal\\'), 'portal');
  assert.equal(baseName('/Users/me/portal'), 'portal');
});
