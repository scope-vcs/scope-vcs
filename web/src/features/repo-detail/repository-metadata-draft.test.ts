import assert from 'node:assert/strict'
import test from 'node:test'
import { reconcileMetadataDraft } from './repository-metadata-draft'

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
