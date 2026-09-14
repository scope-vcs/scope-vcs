import assert from 'node:assert/strict'
import test from 'node:test'
import { acceptMetadataSave, reconcileMetadataDraft } from './repository-metadata-draft'

const original = { description: 'Original', website_url: '' }
const incoming = { description: 'Updated elsewhere', website_url: 'https://example.com' }

test('pristine metadata follows live changes and saved drafts settle when refresh catches up', () => {
  assert.deepEqual(reconcileMetadataDraft({ source: original, value: original, conflict: false }, incoming), {
    source: incoming, value: incoming, conflict: false,
  })
  assert.deepEqual(reconcileMetadataDraft({ source: original, value: incoming, conflict: false }, incoming), {
    source: incoming, value: incoming, conflict: false,
  })
})

test('a successful save becomes the baseline for later edits and live changes', () => {
  const saved = { description: 'Saved', website_url: 'https://saved.example' }
  const settled = acceptMetadataSave(saved)
  assert.deepEqual(settled, { source: saved, value: saved, conflict: false })

  const edited = {
    ...settled,
    value: { ...settled.value, description: 'Edited after save' },
  }
  assert.equal(reconcileMetadataDraft(edited, saved), edited)
  assert.deepEqual(reconcileMetadataDraft(edited, incoming), {
    source: incoming,
    value: edited.value,
    conflict: true,
  })
})
