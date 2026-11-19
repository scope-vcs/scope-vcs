import assert from 'node:assert/strict'
import test from 'node:test'
import { languageSegments, languageSummary } from './repository-languages'

function language(name: string, bytes: number) {
  return { bytes, color: '#123456', name }
}

test('names the six largest languages and folds the rest into Other', () => {
  const segments = languageSegments([
    language('Rust', 5_960),
    language('TypeScript', 2_840),
    language('JavaScript', 850),
    language('HTML', 100),
    language('SQL', 100),
    language('Python', 70),
    language('Shell', 50),
    language('CSS', 30),
  ])

  assert.deepEqual(
    segments.map(({ color, name, percent }) => [name, percent, color]),
    [
      ['Rust', '59.6%', '#123456'],
      ['TypeScript', '28.4%', '#123456'],
      ['JavaScript', '8.5%', '#123456'],
      ['HTML', '1.0%', '#123456'],
      ['SQL', '1.0%', '#123456'],
      ['Python', '0.7%', '#123456'],
      ['Other', '0.8%', null],
    ],
  )
  assert.equal(segments.reduce((share, segment) => share + segment.share, 0).toFixed(6), '1.000000')
})

test('names a seventh language instead of an Other that holds only it', () => {
  const names = ['Rust', 'Go', 'C', 'Lua', 'Zig', 'Nix', 'Shell']
  const segments = languageSegments(names.map((name, index) => language(name, index === 6 ? 1 : 100_000)))

  assert.deepEqual(segments.map(({ name }) => name), names)
  assert.equal(languageSummary(segments.slice(5)), 'Languages: Nix 16.7%, Shell <0.1%')
})
