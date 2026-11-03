import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  activeRepoSection,
  pendingRepoSections,
  REPO_SECTIONS,
  repoSectionsForActor,
} from './repo-section-model'

describe('repository sections', () => {
  it('orders Runs after Requests for owners and members', () => {
    const expected = ['Home', 'Requests', 'Runs', 'Settings']
    assert.deepEqual(
      repoSectionsForActor('Owner').map(({ label }) => label),
      expected,
    )
    assert.deepEqual(
      repoSectionsForActor('Member').map(({ label }) => label),
      expected,
    )
  })

  it('hides Runs and Settings from public actors', () => {
    assert.deepEqual(
      repoSectionsForActor('Public').map(({ label }) => label),
      ['Home', 'Requests'],
    )
  })

  it('keeps the signed-in section list while the repository loads', () => {
    assert.deepEqual(pendingRepoSections(true), REPO_SECTIONS)
    assert.deepEqual(pendingRepoSections(false), repoSectionsForActor('Public'))
  })

  it('recognizes every sibling route before falling back to Home', () => {
    for (const section of REPO_SECTIONS.filter(({ key }) => key !== 'home')) {
      assert.equal(
        activeRepoSection((route) => route === section.to),
        section.key,
      )
    }
    assert.equal(activeRepoSection(() => false), 'home')
  })
})
