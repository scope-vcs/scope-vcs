import assert from 'node:assert/strict'
import test from 'node:test'
import {
  requestWorkspaceWidthFromDrag,
  requestWorkspaceWidthFromKey,
} from './request-workspace-width'

test('dragging through the minimum collapses and the rail edge reopens the sidebar', () => {
  const expanded = { startedCollapsed: false, width: 240, x: 240 }
  assert.deepEqual(requestWorkspaceWidthFromDrag(expanded, 200), { collapsed: false, width: 200 })
  assert.deepEqual(requestWorkspaceWidthFromDrag(expanded, 140), { collapsed: true, width: 240 })

  const collapsed = { startedCollapsed: true, width: 54, x: 54 }
  assert.equal(requestWorkspaceWidthFromDrag(collapsed, 80), null)
  assert.deepEqual(requestWorkspaceWidthFromDrag(collapsed, 86), { collapsed: false, width: 180 })
  assert.deepEqual(requestWorkspaceWidthFromDrag(collapsed, 126), { collapsed: false, width: 220 })
})

test('keyboard resizing reaches the collapsed rail and reopens from it', () => {
  assert.deepEqual(requestWorkspaceWidthFromKey({ collapsed: false, width: 180 }, 'ArrowLeft'), {
    collapsed: true,
    width: 180,
  })
  assert.deepEqual(requestWorkspaceWidthFromKey({ collapsed: true, width: 180 }, 'ArrowRight'), {
    collapsed: false,
    width: 180,
  })
  assert.deepEqual(requestWorkspaceWidthFromKey({ collapsed: true, width: 180 }, 'End'), {
    collapsed: false,
    width: 360,
  })
})
