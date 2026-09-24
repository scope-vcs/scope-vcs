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
export const installCommandAside = "  # go on, we'll wait"

/** Scope's own repository, matching the visibility rules in `.scope/repo.json`. */
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
  kept: ['deploy/', 'docs/', 'legal/', 'AGENTS.md'],
}

/** Notes only the lens shows. Keys name where each one sits on the page. */
export const notes = {
  nav: 'hold the mouse down. press L to put the lens away',
  heroTop: "yes, those are scope's real private folders. you can look. you can't clone",
  cta: 'you found one. eight more are hiding somewhere on this page',
  repo: 'AGENTS.md stays private. it has some choice language in it',
  merge: 'you also get the merge button. pressing it never gets old',
  corner: 'legal/ is 12,250 lines of licenses. someone should probably read it',
  // Non-breaking hyphens keep the folder name on one line.
  install: 'great for the folder named final\u2011final\u2011v2',
  footer: 'an earlier draft of this page was about a sourdough starter. yikes',
} as const

export type NoteId = keyof typeof notes | 'command'
export const touchNavNote = 'drag the ring'
