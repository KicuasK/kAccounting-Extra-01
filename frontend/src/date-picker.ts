export type Day = { year: number; month: number; day: number }
let closeActive: (() => void) | null = null
let nextId = 0
const attached = new WeakSet<HTMLInputElement>()
function dateOf(day: Day): Date { const date = new Date(0); date.setFullYear(day.year, day.month - 1, day.day); date.setHours(12, 0, 0, 0); return date }
function parts(date: Date): Day { return { year: date.getFullYear(), month: date.getMonth() + 1, day: date.getDate() } }
export function iso(day: Day): string { return `${String(day.year).padStart(4, '0')}-${String(day.month).padStart(2, '0')}-${String(day.day).padStart(2, '0')}` }
export function parse(value: string): Day | null { if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return null; const [year, month, day] = value.split('-').map(Number); const result = { year: year!, month: month!, day: day! }; return result.year > 0 && iso(parts(dateOf(result))) === value ? result : null }
export function shift(day: Day, count: number): Day { const date = dateOf(day); date.setDate(date.getDate() + count); return parts(date) }
export function monthShift(day: Day, count: number): Day { const date = dateOf({ ...day, day: 1 }); date.setMonth(date.getMonth() + count); const last = dateOf({ ...parts(date), day: 1 }); last.setMonth(last.getMonth() + 1); last.setDate(0); date.setDate(Math.min(day.day, last.getDate())); return parts(date) }
export function dateInRange(day: Day, lower: Day | null, upper: Day | null): boolean { return day.year >= 1 && day.year <= 9999 && (!lower || iso(day) >= iso(lower)) && (!upper || iso(day) <= iso(upper)) }
function button(text: string, className: string, label?: string): HTMLButtonElement { const node = document.createElement('button'); node.type = 'button'; node.className = className; node.textContent = text; if (label) node.setAttribute('aria-label', label); return node }

/** Adds a large calendar while retaining the native date input and its validation. */
export function attachDatePicker(input: HTMLInputElement): void {
  if (!input.ownerDocument?.defaultView || attached.has(input) || input.type !== 'date' || !input.parentNode) return
  attached.add(input)
  const wrapper = document.createElement('span'); wrapper.className = 'date-picker-field'
  input.parentNode.insertBefore(wrapper, input); wrapper.append(input)
  input.classList.add('date-picker-input')
  const trigger = button('▦', 'date-picker-trigger', '開啟日期選擇器')
  trigger.setAttribute('aria-haspopup', 'dialog'); trigger.setAttribute('aria-expanded', 'false'); wrapper.append(trigger)
  const syncDisabled = (): void => { trigger.disabled = input.disabled || input.readOnly }
  syncDisabled()
  // Node-local observation is collected with this detached input; no document listener stays installed.
  new MutationObserver(syncDisabled).observe(input, { attributes: true, attributeFilter: ['disabled', 'readonly'] })
  trigger.addEventListener('click', () => {
    if (input.disabled || input.readOnly || !input.isConnected) return
    if (trigger.getAttribute('aria-expanded') === 'true') { closeActive?.(); return }
    closeActive?.()
    const panel = document.createElement('div'); panel.className = 'date-picker-panel'; panel.setAttribute('role', 'dialog')
    const titleId = `date-picker-title-${++nextId}`; panel.id = `${titleId}-panel`; panel.setAttribute('aria-labelledby', titleId)
    trigger.setAttribute('aria-controls', panel.id); trigger.setAttribute('aria-expanded', 'true')
    const lower = parse(input.min), upper = parse(input.max)
    const valid = (day: Day): boolean => dateInRange(day, lower, upper)
    const clamp = (day: Day): Day => lower && iso(day) < iso(lower) ? lower : upper && iso(day) > iso(upper) ? upper : day
    let focused = clamp(parse(input.value) ?? parts(new Date()))
    let displayed = { ...focused, day: 1 }
    const header = document.createElement('div'); header.className = 'date-picker-header'
    const title = document.createElement('h3'); title.id = titleId; title.setAttribute('aria-live', 'polite')
    const previous = button('‹', 'date-picker-previous', '上一個月'), next = button('›', 'date-picker-next', '下一個月')
    header.append(previous, title, next)
    const grid = document.createElement('div'); grid.className = 'date-picker-grid'; grid.setAttribute('role', 'grid'); grid.setAttribute('aria-labelledby', titleId)
    const footer = document.createElement('div'); footer.className = 'date-picker-footer'
    const today = button('今天', 'date-picker-today'), clear = button('清除日期', 'date-picker-clear'), done = button('關閉', 'date-picker-close')
    footer.append(today, clear, done); panel.append(header, grid, footer); document.body.append(panel)
    let closed = false
    const close = (restore = false): void => {
      if (closed) return; closed = true
      observer.disconnect(); document.removeEventListener('mousedown', outside); document.removeEventListener('touchstart', outside); document.removeEventListener('focusin', focusOutside)
      document.removeEventListener('keydown', escape); window.removeEventListener('resize', position); window.removeEventListener('scroll', position, true)
      panel.remove(); trigger.setAttribute('aria-expanded', 'false'); trigger.removeAttribute('aria-controls'); closeActive = null
      if (restore && trigger.isConnected) trigger.focus()
    }
    closeActive = () => close()
    const choose = (value: string): void => {
      if (input.disabled || input.readOnly || !input.isConnected) { close(); return }
      const day = parse(value); if (value && (!day || !valid(day))) return
      input.value = value; close(true); input.dispatchEvent(new Event('input', { bubbles: true })); input.dispatchEvent(new Event('change', { bubbles: true }))
    }
    function position(): void {
      if (!input.isConnected || input.disabled || input.readOnly) { close(); return }
      const rect = wrapper.getBoundingClientRect(); const width = panel.offsetWidth; const height = panel.offsetHeight
      panel.style.left = `${Math.max(12, Math.min(rect.left, window.innerWidth - width - 12))}px`
      panel.style.top = `${Math.max(12, rect.bottom + height + 8 <= window.innerHeight ? rect.bottom + 8 : rect.top - height - 8)}px`
    }
    function render(moveFocus = false): void {
      title.textContent = `${displayed.year} 年 ${displayed.month} 月`; grid.replaceChildren()
      const weekday = document.createElement('div'); weekday.className = 'date-picker-week'; weekday.setAttribute('role', 'row')
      ;['日', '一', '二', '三', '四', '五', '六'].forEach(text => { const cell = document.createElement('span'); cell.textContent = text; cell.setAttribute('role', 'columnheader'); weekday.append(cell) }); grid.append(weekday)
      const start = shift(displayed, -dateOf(displayed).getDay()); const current = iso(parts(new Date()))
      for (let week = 0; week < 6; week++) {
        const row = document.createElement('div'); row.className = 'date-picker-week'; row.setAttribute('role', 'row')
        for (let column = 0; column < 7; column++) {
          const day = shift(start, week * 7 + column); const value = iso(day)
          const cell = document.createElement('span'); cell.setAttribute('role', 'gridcell'); cell.setAttribute('aria-selected', String(value === input.value))
          const control = button(String(day.day), 'date-picker-day', `${day.year} 年 ${day.month} 月 ${day.day} 日`)
          control.dataset.date = value; control.disabled = !valid(day); control.tabIndex = value === iso(focused) ? 0 : -1
          if (day.month !== displayed.month) control.classList.add('is-outside-month')
          if (value === input.value) control.classList.add('is-selected')
          if (value === current) control.setAttribute('aria-current', 'date')
          control.addEventListener('click', () => choose(value)); control.addEventListener('keydown', event => navigate(event, day)); cell.append(control); row.append(cell)
        }
        grid.append(row)
      }
      const prevMonth = monthShift(displayed, -1), nextMonth = monthShift(displayed, 1)
      previous.disabled = prevMonth.year < 1 || Boolean(lower && iso(shift(displayed, -1)) < iso(lower))
      next.disabled = nextMonth.year > 9999 || Boolean(upper && iso(nextMonth) > iso(upper))
      today.disabled = !valid(parts(new Date())); position()
      if (moveFocus) grid.querySelector<HTMLButtonElement>('button[tabindex="0"]')?.focus()
    }
    function navigate(event: KeyboardEvent, day: Day): void {
      let nextDay: Day
      if (event.key === 'ArrowLeft') nextDay = shift(day, -1)
      else if (event.key === 'ArrowRight') nextDay = shift(day, 1)
      else if (event.key === 'ArrowUp') nextDay = shift(day, -7)
      else if (event.key === 'ArrowDown') nextDay = shift(day, 7)
      else if (event.key === 'Home') nextDay = shift(day, -dateOf(day).getDay())
      else if (event.key === 'End') nextDay = shift(day, 6 - dateOf(day).getDay())
      else if (event.key === 'PageUp') nextDay = monthShift(day, event.shiftKey ? -12 : -1)
      else if (event.key === 'PageDown') nextDay = monthShift(day, event.shiftKey ? 12 : 1)
      else return
      event.preventDefault(); nextDay = clamp(nextDay); if (!valid(nextDay)) return
      focused = nextDay; displayed = { ...nextDay, day: 1 }; render(true)
    }
    previous.addEventListener('click', () => { focused = clamp(monthShift(focused, -1)); displayed = { ...focused, day: 1 }; render() })
    next.addEventListener('click', () => { focused = clamp(monthShift(focused, 1)); displayed = { ...focused, day: 1 }; render() })
    today.addEventListener('click', () => choose(iso(parts(new Date())))); clear.addEventListener('click', () => choose('')); done.addEventListener('click', () => close(true))
    const contains = (target: EventTarget | null): boolean => target instanceof Node && (panel.contains(target) || wrapper.contains(target))
    function outside(event: Event): void { if (!contains(event.target)) close() }
    function focusOutside(event: FocusEvent): void { if (!contains(event.target)) close() }
    function escape(event: KeyboardEvent): void { if (event.key === 'Escape') { event.preventDefault(); close(true) } }
    const observer = new MutationObserver(() => { if (!input.isConnected || input.disabled || input.readOnly || input.min !== (lower ? iso(lower) : '') || input.max !== (upper ? iso(upper) : '')) close() })
    observer.observe(document.body, { childList: true, subtree: true }); observer.observe(input, { attributes: true, attributeFilter: ['disabled', 'readonly', 'min', 'max'] })
    document.addEventListener('mousedown', outside); document.addEventListener('touchstart', outside); document.addEventListener('focusin', focusOutside); document.addEventListener('keydown', escape)
    window.addEventListener('resize', position); window.addEventListener('scroll', position, true); render(true)
  })
}
