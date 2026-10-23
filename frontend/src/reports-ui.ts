import { parseReport, reportDate, reportAccountOptions, type ReportView, type ReportQuery } from './reports-model.ts'
import { parseAccountTree } from './account-tree.ts'
import { ApiError, apiGuidance } from './session-client.ts'
export type { ReportView } from './reports-model.ts'
export interface ReportsContext {
  getSelection(): { bookId: string; selectionId: string } | null
  request(path: string, signal: AbortSignal, method?: string, payload?: unknown): Promise<unknown>
  onChanged(): void
  onSelectionChanged(): void
  createAccountName(name: string): HTMLElement
  onOpenPosted?(id: string): void
}
const titles: Record<ReportView,string> = { 'general-journal':'普通日記簿',ledger:'科目分類帳','trial-balance':'試算表','balance-sheet':'資產負債表','income-expense':'收支損益表' }
function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = ''): HTMLElementTagNameMap[K] {
  const result=document.createElement(tag);result.textContent=text;result.className=className;return result
}
export function mountReportsUI(host: HTMLElement, context: ReportsContext, view: ReportView): { dispose(): void; hasUnsavedChanges(): false } {
  const root=node('section','','reports-ui'),controls=node('form','','report-controls'),status=node('p','','report-status'),output=node('div'),summary=node('footer','','report-summary')
  const today=new Date(), date=`${today.getFullYear()}-${String(today.getMonth()+1).padStart(2,'0')}-${String(today.getDate()).padStart(2,'0')}`
  const from=node('input'),through=node('input'),account=node('select'),run=node('button','查詢')
  from.type='date';from.value=`${date.slice(0,4)}-01-01`;through.type='date';through.value=date;run.type='submit'
  const asOf=view==='trial-balance'||view==='balance-sheet'
  function label(text: string, input: HTMLElement): void { const el=node('label',text);el.append(input);controls.append(el) }
  if(!asOf) label('起日',from)
  label(asOf?'截至日期':'迄日',through)
  if(view==='ledger') { account.append(node('option','請選擇科目'));account.options[0]!.value='';label('科目',account) }
  controls.append(run);status.setAttribute('role','status');root.append(node('h2',titles[view]),node('p','僅含已過帳分錄；日期範圍含起迄日。金額以最小貨幣單位顯示。'),controls,status,output,summary);host.append(root)
  let disposed=false,version=0,controller: AbortController | null=null
  const accountsController=new AbortController()
  const selection=context.getSelection(),isCurrent=(): boolean => { const current=context.getSelection();return !disposed&&!!selection&&current?.bookId===selection.bookId&&current.selectionId===selection.selectionId }
  function clear(): void { output.replaceChildren();summary.replaceChildren() }
  function showError(error: unknown): void {
    if(error instanceof ApiError && error.code==='selection_changed') { context.onSelectionChanged();return }
    status.textContent=error instanceof ApiError?apiGuidance(error):'報表無法安全讀取或核對，未顯示資料；請確認服務後重新查詢。'
  }
  async function load(): Promise<void> {
    controller?.abort();controller=new AbortController();const active=controller,requestVersion=++version;clear()
    if(!isCurrent()) {status.textContent='請重新選擇帳簿後開啟報表';return}
    const query: ReportQuery={from:asOf?through.value:from.value,through:through.value}
    if(!reportDate(query.from)||!reportDate(query.through)||query.from>query.through) {status.textContent='請輸入有效日期，起日不得晚於迄日';return}
    if(view==='ledger') {if(!account.value) {status.textContent='請選擇科目';return} query.accountId=account.value}
    const path=`/api/reports/${view}/${view==='ledger'?`${account.value}/`:''}${asOf?query.through:`${query.from}/${query.through}`}`
    status.textContent='讀取報表…';run.disabled=true
    try {
      const raw=await context.request(path,active.signal)
      if(!isCurrent()||requestVersion!==version) return
      const model=parseReport(view,raw,query),table=node('table','','report-table'),head=node('thead'),headRow=node('tr'),body=node('tbody')
      for(const name of model.columns) {const th=node('th',name);th.scope='col';headRow.append(th)}head.append(headRow)
      for(const cells of model.rows) {const row=node('tr');for(const cell of cells) {const td=node('td',cell.account||(cell.entryId&&context.onOpenPosted)?'':cell.text,cell.amount?'amount':'');if(cell.account) td.append(context.createAccountName(cell.text));if(cell.entryId&&context.onOpenPosted) {const button=node('button',cell.text);button.type='button';const entryId=cell.entryId;button.addEventListener('click',()=>{if(isCurrent())context.onOpenPosted?.(entryId)});td.append(button)}row.append(td)}body.append(row)}
      table.append(node('caption',`${titles[view]} · ${asOf?query.through:`${query.from} 至 ${query.through}`} · 已過帳`),head,body);output.append(table);for(const text of model.summary) summary.append(node('span',text))
      status.textContent=`${asOf?`截至 ${query.through}`:`${query.from} 至 ${query.through}`} · ${model.rows.length} 列${model.rows.length===0?' · 期間無已過帳資料':''}`
    } catch(error) { if(!active.signal.aborted&&isCurrent()&&requestVersion===version) {clear();showError(error)} }
    finally {if(requestVersion===version&&!disposed) run.disabled=false}
  }
  const submit=(event: SubmitEvent): void => {event.preventDefault();void load()};controls.addEventListener('submit',submit)
  const invalidate=(): void => {version++;controller?.abort();clear();run.disabled=false;status.textContent='條件已變更，請按查詢'}
  controls.addEventListener('change',invalidate)
  if(view==='ledger') {
    run.disabled=true
    void context.request('/api/accounts/tree',accountsController.signal).then(value=>{
      if(!isCurrent()) return
      for(const row of reportAccountOptions(parseAccountTree(value))) {const option=node('option',row.label);option.value=row.id;account.append(option)}
      status.textContent='選擇科目及日期後查詢';run.disabled=false
    }).catch(error=>{if(isCurrent()) showError(error)})
  } else void load()
  return {dispose(): void {disposed=true;version++;controller?.abort();accountsController.abort();controls.removeEventListener('submit',submit);controls.removeEventListener('change',invalidate);root.remove()},hasUnsavedChanges:()=>false}
}
