// Made-up sessions, used when the page runs in a plain browser instead of the app.

export function demoSnapshot(now = Date.now()) {
  const m = 60_000;
  let n = 0;
  const S = (id, title, state, label, doing, mins, extra = {}) => ({
    id: `local_${id}`, title, state, label, doing, helpers: 0, folder: '/made/up/' + id,
    createdAt: now - 1000 * m + n++, lastUsed: now - (mins + 1) * m, lastActivity: now - mins * m, turnEnd: now - mins * m,
    unread: false, question: null, files: [], steps: [], ...extra,
  });
  const steps = (...xs) => xs.map((doing, i) => ({ doing, at: now - (xs.length - i) * 2 * m }));
  return {
    groupSource: 'sidebar', groupNote: '', totalRecords: 14, archivedHidden: 2, sessionsRoot: '(demo)',
    groups: [
      { id: 'g1', name: 'Client portal', colour: '', folder: 'C:\\code\\portal', sessions: [
        S('login', 'Fix login redirect', 'needs', 'Needs you', 'Asking you: run the migration on staging first?', 12, {
          question: { text: 'Run the migration on staging first?', options: ['Yes, staging first', 'No, local database only'] },
          files: [{ path: 'C:\\code\\portal\\src\\auth\\redirect.ts', name: 'redirect.ts', added: 18, removed: 6 }, { path: 'C:\\code\\portal\\migrations\\0042.sql', name: '0042.sql', added: 24, removed: 0 }],
          steps: steps('Reading middleware.ts', 'Searching for "redirect"', 'Editing redirect.ts', 'Writing 0042.sql', 'Asking you: run the migration on staging first?') }),
        S('invoices', 'Invoice PDF export', 'working', 'Quiet', 'Editing layout.ts, or waiting for your approval', 2, { steps: steps('Reading export.ts', 'Editing layout.ts') }),
        S('search', 'Search filters', 'working', 'Working', 'Run the test suite', 1, { helpers: 2, files: [{ path: 'src/search/filters.ts', name: 'filters.ts', added: 61, removed: 22 }], steps: steps('Editing filters.ts', 'Briefing a helper: write tests', 'Run the test suite') }),
        S('a11y', 'Accessibility pass', 'idle', 'Idle', 'Fixed the focus ring and contrast', 1500),
      ] },
      { id: 'g2', name: 'Data pipeline', colour: '', folder: 'C:\\code\\pipeline', sessions: [
        S('nightly', 'Nightly load', 'failed', 'Error', 'Stopped on an API error', 25, { steps: steps('Running pytest', 'Running pytest') }),
        S('schema', 'Schema v4', 'working', 'Working', 'Writing v4.sql', 1, { files: [{ path: 'schema/v4.sql', name: 'v4.sql', added: 140, removed: 0 }] }),
        S('dedupe', 'Dedupe customers', 'idle', 'Idle', 'Done. Merge rule added, PR open and checks passed', 40, { unread: true, files: [{ path: 'jobs/dedupe.py', name: 'dedupe.py', added: 34, removed: 8 }], steps: steps('Editing dedupe.py', 'Running pytest', 'Running gh pr create') }),
      ] },
      { id: 'g3', name: 'Internal tools', colour: '', folder: '', sessions: [
        S('report', 'Weekly report', 'working', 'Working', 'Reading numbers.csv', 3),
        S('bot', 'Standup bot', 'idle', 'Stopped', 'Stopped partway through a task', 4000),
        S('cleanup', 'Branch cleanup', 'idle', 'Idle', 'Deleted 14 merged branches', 5, { unread: true }),
      ] },
      { id: 'g4', name: 'Docs', colour: '', folder: '', sessions: [
        S('onboarding', 'Onboarding guide', 'idle', 'Idle', 'Rewrote sections 2 and 3', 90, { unread: true, files: [{ path: 'docs/onboarding.md', name: 'onboarding.md', added: 96, removed: 51 }] }),
        S('api', 'API guide', 'working', 'Working', 'Checking every example still runs', 3),
      ] },
    ],
  };
}
