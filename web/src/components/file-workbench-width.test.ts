import assert from 'node:assert/strict'
import test from 'node:test'
import { clampFilePaneWidth, filePaneKeyboardWidth } from './file-workbench-width'

test('pointer resizing preserves the document space and a usable file pane', () => {
  assert.equal(clampFilePaneWidth(250 + 500), 360)
  assert.equal(clampFilePaneWidth(250 - 500), 180)
  assert.equal(clampFilePaneWidth(250 + 24), 274)
})

test('keyboard resizing supports arrows and endpoints without consuming other keys', () => {
  assert.equal(filePaneKeyboardWidth(250, 'ArrowLeft'), 240)
  assert.equal(filePaneKeyboardWidth(250, 'ArrowRight'), 260)
  assert.equal(filePaneKeyboardWidth(180, 'ArrowLeft'), 180)
  assert.equal(filePaneKeyboardWidth(360, 'ArrowRight'), 360)
  assert.equal(filePaneKeyboardWidth(250, 'Home'), 180)
  assert.equal(filePaneKeyboardWidth(250, 'End'), 360)
  assert.equal(filePaneKeyboardWidth(250, 'Tab'), null)
})
