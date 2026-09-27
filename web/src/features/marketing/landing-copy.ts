/** Everything the landing page says, in both views. The private view is what
 * the lens reveals; each pair must fit the same space as its public text. */
export type LandingView = 'public' | 'private'
export type PairedText = Record<LandingView, string>

export const sourceUrl = 'https://scopevcs.com/adamblumoff/scope-vcs'

export const heroCopy = {
  title: 'One repository.',
  subtitle: { public: 'Part of it is public.', private: 'All of it is yours.' },
  lede: { public: 'Contributors clone only what you share.', private: 'Private paths never leave your team.' },
} satisfies Record<string, string | PairedText>

export const mergeTitle: PairedText = { public: 'Anyone can send a change.', private: 'Only you can merge it.' }
export const installTitle: PairedText = { public: 'Bring your repository.', private: 'Private paths included.' }

/** Scope's own repository. `kept` must match what's private on scopevcs.com. */
export const repoPanel = {
  name: 'adamblumoff/scope-vcs',
  shared: [
    { name: 'api/', age: '1h' },
    { name: 'cli/', age: '3h' },
    { name: 'crates/', age: '1h' },
    { name: 'web/', age: '20m' },
    { name: 'worker/', age: '2d' },
    { name: 'README.md', age: '1w' },
  ],
  kept: ['.scope/images/', '.scope/runs/'],
}

/** Notes only the lens shows. Keys name where each one sits on the page. */
export const notes = {
  nav: 'hold the mouse down to reveal everything. press L to put the lens away',
  heroTop: "yes, those are scope's real private folders",
  cta: 'you found one. a surprise is waiting if you find them all',
  repo: 'AGENTS.md used to be private. it may or may not have contained poor language',
  graph: "this diagram is not an accurate representation of a normal workflow",
  merge: 'if you want to merge, great. otherwise, snooze it for when you feel like dealing with it',
  corner: 'someone should probably read legal, seems important',
  // Non-breaking hyphens keep the folder name on one line.
  install: 'great for the folder named final\u2011final\u2011v2',
  footer: 'an earlier draft of this page was about a sourdough starter. great work, opus',
} as const

export type NoteId = keyof typeof notes
export const touchNavNote = 'drag the ring'
