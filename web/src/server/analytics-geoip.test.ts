import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { convertHogToJS, execSync } from '@posthog/hogvm'
import test, { mock } from 'node:test'

const source = readFileSync('../dev/analytics/geoip.hog')
const compiled = JSON.parse(readFileSync('../dev/analytics/geoip.bytecode.json', 'utf8'))

test('GeoIP bytecode is bound to the checked-in Hog source', () => {
  assert.equal(createHash('sha256').update(source).digest('hex'), compiled.sourceSha256,
    'Configure the test project and run configure-geoip.mjs --export-bytecode after changing geoip.hog')
})

test('GeoIP executes coarse-only enrichment and strips forged locations without person mutations', () => {
  for (const fixture of [
    { name: 'enabled', source: 'browser', disabled: false, ip: '89.160.20.129', lookup: true },
    { name: 'disabled', source: 'browser', disabled: true, ip: '89.160.20.129', lookup: false },
    { name: 'missing IP', source: 'browser', disabled: false, lookup: false },
    { name: 'backend', source: 'api', disabled: false, ip: '89.160.20.129', lookup: false },
    { name: 'missing opt-in', source: 'browser', ip: '89.160.20.129', lookup: false },
    { name: 'lookup miss', source: 'browser', disabled: false, ip: '89.160.20.129', lookup: true, missing: true },
  ]) {
    const geoipLookup = mock.fn(() => fixture.missing ? null : {
      country: { names: { en: 'Sweden' }, isoCode: 'SE' },
      city: { names: { en: 'Lidköping' } },
      subdivisions: [
        { names: { en: 'Västra Götaland County' }, isoCode: 'O' },
        { names: { en: 'Unwanted second subdivision' }, isoCode: 'X' },
      ],
      postal: { code: '53100' },
      location: { latitude: 58.5, longitude: 13.1, accuracyRadius: 20, timeZone: 'Europe/Stockholm' },
    })
    const print = mock.fn()
    const properties = {
      source: fixture.source, route_name: 'home', distinct_id: 'visitor',
      ...(fixture.disabled !== undefined ? { $geoip_disable: fixture.disabled } : {}),
      ...(fixture.ip ? { $ip: fixture.ip } : {}),
      $geoip_country_name: 'Forged country', $geoip_city_name: 'Forged city',
      $geoip_latitude: 12, $geoip_longitude: 34, $geoip_postal_code: 'secret',
      $initial_geoip_country_name: 'Forged initial country', $geoip_future_field: 'secret',
      $set: { $geoip_latitude: 12 }, $set_once: { $initial_geoip_longitude: 34 },
      $unset: ['private-field'],
    }
    const event = { event: '$pageview', uuid: 'event-id', properties }
    const result = convertHogToJS(execSync(compiled.bytecode, {
      globals: { event }, functions: { geoipLookup, print },
    }))
    assert.deepEqual(result, {
      event: '$pageview', uuid: 'event-id', properties: {
        source: fixture.source, route_name: 'home', distinct_id: 'visitor',
        ...(fixture.disabled !== undefined ? { $geoip_disable: fixture.disabled } : {}),
        ...(fixture.lookup && !fixture.missing ? {
          $geoip_country_name: 'Sweden', $geoip_country_code: 'SE', $geoip_city_name: 'Lidköping',
          $geoip_subdivision_1_name: 'Västra Götaland County', $geoip_subdivision_1_code: 'O',
        } : {}),
      },
    }, fixture.name)
    assert.equal(geoipLookup.mock.callCount(), fixture.lookup ? 1 : 0, fixture.name)
    if (fixture.lookup) assert.deepEqual(geoipLookup.mock.calls[0].arguments, [fixture.ip])
    assert.equal(print.mock.callCount(), 0, fixture.name)
  }
})
