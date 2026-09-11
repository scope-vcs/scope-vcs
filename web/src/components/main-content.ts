export const MAIN_CONTENT_ID = 'main-content'

// The app shell's scrolling region; feature views adjust its scroll position
// when they restore or extend content above the fold.
export function mainScrollContainer() {
  return document.getElementById(MAIN_CONTENT_ID)
}
