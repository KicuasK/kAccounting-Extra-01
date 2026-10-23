import assert from 'node:assert/strict'
import test from 'node:test'
import { parse, iso, shift, monthShift, dateInRange } from '../src/date-picker.ts'

function day(value: string) { const result = parse(value); assert.ok(result, value); return result }

test('calendar validates Gregorian leap years and rejects malformed or normalized dates', () => {
  assert.equal(iso(day('2000-02-29')), '2000-02-29')
  assert.equal(iso(day('2028-02-29')), '2028-02-29')
  for (const value of ['2100-02-29', '2027-02-29', '2026-04-31', '2026-00-01', '2026-13-01', '2026-01-00', '2026-01-32', '2026-1-01', '26-01-01', '2026-01-01T00:00:00', ' 2026-01-01', '', '0000-01-01', '10000-01-01']) {
    assert.equal(parse(value), null, value)
  }
})

test('calendar preserves early years and both supported year boundaries', () => {
  for (const value of ['0001-01-01', '0099-12-31', '0100-01-01', '9999-12-31']) {
    assert.equal(iso(day(value)), value)
    assert.equal(dateInRange(day(value), null, null), true)
  }
  assert.equal(dateInRange(shift(day('0001-01-01'), -1), null, null), false)
  assert.equal(dateInRange(shift(day('9999-12-31'), 1), null, null), false)
})

test('month navigation clamps the day at month end and crosses years', () => {
  assert.equal(iso(monthShift(day('2028-01-31'), 1)), '2028-02-29')
  assert.equal(iso(monthShift(day('2100-01-31'), 1)), '2100-02-28')
  assert.equal(iso(monthShift(day('2028-03-31'), -1)), '2028-02-29')
  assert.equal(iso(monthShift(day('2028-02-29'), 12)), '2029-02-28')
  assert.equal(iso(monthShift(day('2026-12-15'), 1)), '2027-01-15')
  assert.equal(iso(monthShift(day('2026-01-15'), -1)), '2025-12-15')
})

test('day and week navigation cross leap days, months, and years correctly', () => {
  assert.equal(iso(shift(day('2000-02-28'), 1)), '2000-02-29')
  assert.equal(iso(shift(day('2100-02-28'), 1)), '2100-03-01')
  assert.equal(iso(shift(day('2026-01-01'), -1)), '2025-12-31')
  assert.equal(iso(shift(day('2028-02-25'), 7)), '2028-03-03')
  assert.equal(iso(shift(day('2028-03-03'), -7)), '2028-02-25')
  assert.equal(iso(shift(day('0099-12-31'), 1)), '0100-01-01')
})

test('date bounds are inclusive and support one-sided ranges', () => {
  const lower = day('2028-02-28'), upper = day('2028-03-01')
  for (const value of ['2028-02-28', '2028-02-29', '2028-03-01']) assert.equal(dateInRange(day(value), lower, upper), true)
  assert.equal(dateInRange(day('2028-02-27'), lower, upper), false)
  assert.equal(dateInRange(day('2028-03-02'), lower, upper), false)
  assert.equal(dateInRange(day('0001-01-01'), null, upper), true)
  assert.equal(dateInRange(day('9999-12-31'), lower, null), true)
  assert.equal(dateInRange(lower, lower, lower), true)
  assert.equal(dateInRange(upper, lower, lower), false)
})
