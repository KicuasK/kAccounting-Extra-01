import { validTime } from './journal-model.ts'
import { isPositiveI64Decimal, parseAccountTree, flattenAccountTree, type AccountNode } from './account-tree.ts'
export type ReportView = 'general-journal' | 'ledger' | 'trial-balance' | 'balance-sheet' | 'income-expense'
export interface ReportQuery { from: string; through: string; accountId?: string }
export interface ReportCell { text: string; account?: boolean; amount?: boolean; entryId?: string }
export interface ReportTable { columns: string[]; rows: ReportCell[][]; summary: string[] }
const fail = (): never => { throw new Error('報表資料格式或核對結果不符，未顯示資料') }
const rec = (v: unknown): Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v) ? v as Record<string, unknown> : fail()
const arr = (v: unknown): unknown[] => Array.isArray(v) && v.length <= 100_000 ? v : fail()
const str = (v: unknown): string => typeof v === 'string' && v.length <= 100_000 ? v : fail()
const optionalText = (v: unknown): string => v === null ? '' : str(v)
const id = (v: unknown): string => typeof v === 'string' && isPositiveI64Decimal(v) ? v : fail()
export function reportMoney(v: unknown, signed = false): bigint {
  if (typeof v !== 'string' || !(signed ? /^(0|-?[1-9]\d*)$/ : /^(0|[1-9]\d*)$/).test(v) || v.length > 20) return fail()
  const n = BigInt(v)
  return n <= 9223372036854775807n && n >= -9223372036854775808n ? n : fail()
}
export function reportDate(v: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(v) || v < '0001-01-01') return false
  const d = new Date(`${v}T00:00:00Z`)
  return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === v
}
function entryDateTime(date: string, time: unknown): string {
  if (time === undefined || time === null) return date
  if (typeof time !== 'string' || !validTime(time)) return fail()
  return `${date} ${time}`
}
const fmt = (n: bigint): string => n.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',')
const c = (text: string): ReportCell => ({ text })
const a = (text: string): ReportCell => ({ text, account: true })
const m = (n: bigint): ReportCell => ({ text: fmt(n), amount: true })
const element = (v: unknown): string => ['asset','liability','equity','income','expense'].includes(str(v)) ? str(v) : fail()
const names: Record<string,string> = { asset:'資產',liability:'負債',equity:'權益',income:'收益',expense:'費損' }
export function reportAccountOptions(tree: AccountNode[]): { id: string; label: string }[] {
  const labels=new Map<string,string>(),result: {id:string;label:string}[]=[]
  for(const {account} of flattenAccountTree(tree,new Set())) {
    const path=account.parentId ? `${labels.get(account.parentId)!} › ${account.name}` : account.name
    labels.set(account.id,path)
    if(account.kind==='posting') result.push({id:account.id,label:`${names[account.element]} · ${path}`})
  }
  return result
}
function direction(n: bigint, normal: string): ReportCell {
  const debit=(normal==='asset'||normal==='expense')?n:-n
  return {text:debit===0n?'0':`${debit>0n?'借':'貸'} ${fmt(debit<0n?-debit:debit)}`,amount:true}
}
type ReportOrder = { date: string; ids: bigint[] }
function ordered(previous: ReportOrder | null, next: ReportOrder): ReportOrder {
  if(previous) {
    if(next.date<previous.date) fail()
    if(next.date===previous.date) {
      let comparison=0
      for(let i=0;i<next.ids.length;i++) {if(next.ids[i]!<previous.ids[i]!) {comparison=-1;break}if(next.ids[i]!>previous.ids[i]!) {comparison=1;break}}
      if(comparison<=0) fail()
    }
  }
  return next
}
const pair = (r: Record<string,unknown>, prefix: string): [bigint,bigint] => {
  const dr = reportMoney(r[`${prefix}debit_balance_minor`]), cr = reportMoney(r[`${prefix}credit_balance_minor`])
  if (dr !== 0n && cr !== 0n) fail()
  return [dr,cr]
}
export function parseReport(view: ReportView, value: unknown, query: ReportQuery): ReportTable {
  const r = rec(value), rows: ReportCell[][] = [], seen = new Set<string>()
  if (!reportDate(query.from) || !reportDate(query.through) || query.from > query.through) fail()
  if (view === 'trial-balance' || view === 'balance-sheet') { if (r.as_of !== query.through) fail() }
  else if (r.from !== query.from || r.through !== query.through) fail()
  const unique = (v: unknown): string => { const key=id(v); if(seen.has(key)) fail(); seen.add(key); return key }
  let previousAccount=0n
  const account = (v: unknown): Record<string,unknown> => { const row=rec(v); const key=BigInt(unique(row.account_id));if(key<=previousAccount) fail();previousAccount=key; str(row.account_name); element(row.element); return row }
  if (view === 'trial-balance') {
    let dr=0n,cr=0n
    for(const v of arr(r.rows)) { const row=account(v), [d,k]=pair(row,''); if(reportMoney(row.debit_turnover_minor)-reportMoney(row.credit_turnover_minor)!==d-k) fail(); dr+=d;cr+=k; rows.push([a(str(row.account_name)),c(names[str(row.element)]!),m(reportMoney(row.debit_turnover_minor)),m(reportMoney(row.credit_turnover_minor)),m(d),m(k)]) }
    if(dr!==reportMoney(r.total_debit_minor)||cr!==reportMoney(r.total_credit_minor)||dr!==cr) fail()
    return { columns:['科目','性質','借方發生額','貸方發生額','借方餘額','貸方餘額'],rows,summary:[`借方餘額合計 ${fmt(dr)}`,`貸方餘額合計 ${fmt(cr)}`] }
  }
  if(view==='income-expense') {
    let income=0n,expense=0n
    for(const v of arr(r.rows)) { const row=account(v), e=str(row.element); if(e!=='income'&&e!=='expense') fail(); const dr=reportMoney(row.debit_turnover_minor),cr=reportMoney(row.credit_turnover_minor),net=reportMoney(row.net_activity_minor,true); if(net!==(e==='income'?cr-dr:dr-cr)) fail(); if(e==='income') income+=net; else expense+=net; rows.push([a(str(row.account_name)),c(names[e]!),m(dr),m(cr),direction(net,e)]) }
    if(income!==reportMoney(r.total_income_minor,true)||expense!==reportMoney(r.total_expense_minor,true)||income-expense!==reportMoney(r.net_result_minor,true)) fail()
    return { columns:['科目','性質','借方發生額','貸方發生額','本期淨額（借／貸）'],rows,summary:[`收益 ${fmt(income)}`,`費損 ${fmt(expense)}`,`本期淨損益（收益減費損） ${fmt(income-expense)}`] }
  }
  if(view==='balance-sheet') {
    const nodes = new Map<string, { account: Record<string,unknown>; children: unknown[] }>()
    for (const item of arr(r.hierarchy)) { const account=rec(item),key=id(account.id);if(nodes.has(key)) fail();nodes.set(key,{account,children:[]}) }
    const roots: unknown[]=[]
    for(const item of nodes.values()) {if(item.account.parent_id===null) roots.push(item);else {const parent=nodes.get(id(item.account.parent_id));if(!parent) fail();parent!.children.push(item)}}
    const hierarchy=flattenAccountTree(parseAccountTree(roots),new Set())
    if(hierarchy.length!==nodes.size) fail()
    const totals={asset:0n,liability:0n,equity:0n}
    const balances = new Map<string,bigint>()
    for(const v of arr(r.rows)) { const row=account(v),e=str(row.element),source=nodes.get(id(row.account_id))?.account; if(!source||source.kind!=='posting'||source.name!==row.account_name||source.element!==e||!['asset','liability','equity'].includes(e)) fail(); const n=reportMoney(row.balance_minor,true); totals[e as keyof typeof totals]+=n;balances.set(id(row.account_id),n) }
    for(const item of hierarchy) if(['asset','liability','equity'].includes(item.account.element)) {
      const balance=balances.get(item.account.id)
      rows.push([a(`${'　'.repeat(item.depth)}${item.account.name}`),c(names[item.account.element]!),item.account.kind==='posting'?(balance===undefined?c('無已過帳資料'):direction(balance,item.account.element)):c('')])
    }
    const result=reportMoney(r.unclosed_net_result_minor,true),rhs=totals.liability+totals.equity+result
    if(totals.asset!==reportMoney(r.total_assets_minor,true)||totals.liability!==reportMoney(r.total_liabilities_minor,true)||totals.equity!==reportMoney(r.total_equity_minor,true)||rhs!==reportMoney(r.total_liabilities_and_equity_minor,true)||totals.asset!==rhs) fail()
    return { columns:['科目','性質','餘額（借／貸）'],rows,summary:[`資產 ${fmt(totals.asset)}`,`負債 ${fmt(totals.liability)}`,`權益 ${fmt(totals.equity)}`,`未結轉淨損益 ${fmt(result)}`,`負債及權益含未結轉 ${fmt(rhs)}`] }
  }
  if(view==='ledger') {
    if(id(r.account_id)!==query.accountId) fail(); str(r.account_name); element(r.element)
    const [od,oc]=pair(r,'opening_'); let balance=od-oc,dr=0n,cr=0n
    rows.push([c('期初'),c(''),c(''),m(0n),m(0n),m(od),m(oc),c('')])
    let previous: ReportOrder | null=null
    for(const v of arr(r.lines)) { const row=rec(v); const lineId=unique(row.line_id),entryId=id(row.entry_id),lineNo=id(row.line_no); const date=str(row.entry_date); if(!reportDate(date)||date<query.from||date>query.through) fail();previous=ordered(previous,{date,ids:[BigInt(entryId),BigInt(lineNo),BigInt(lineId)]}); const amount=reportMoney(row.amount_minor),side=str(row.side); if(amount===0n||!['debit','credit'].includes(side)) fail(); if(side==='debit') { dr+=amount;balance+=amount } else { cr+=amount;balance-=amount } const [d,k]=pair(row,''); if(d-k!==balance) fail(); rows.push([c(entryDateTime(date,row.entry_time)),c(optionalText(row.reference)),c(optionalText(row.explanation)),m(side==='debit'?amount:0n),m(side==='credit'?amount:0n),m(d),m(k),c(optionalText(row.memo))]) }
    const [d,k]=pair(r,'closing_'); if(d-k!==balance||dr!==reportMoney(r.period_debit_turnover_minor)||cr!==reportMoney(r.period_credit_turnover_minor)) fail()
    return {columns:['日期','傳票號碼','摘要','借方','貸方','借方餘額','貸方餘額','備註'],rows,summary:[`科目 ${str(r.account_name)}`,`本期借方 ${fmt(dr)}`,`本期貸方 ${fmt(cr)}`,`期末借方 ${fmt(d)}`,`期末貸方 ${fmt(k)}`]}
  }
  let totalDr=0n,totalCr=0n
  let previous: ReportOrder | null=null
  const lineIds=new Set<string>()
  for(const v of arr(r.entries)) { const entry=rec(v); const entryId=unique(entry.id); const date=str(entry.entry_date); if(!reportDate(date)||date<query.from||date>query.through||!str(entry.posted_at)||Number.isNaN(Date.parse(str(entry.posted_at)))) fail();previous=ordered(previous,{date,ids:[BigInt(entryId)]}); let dr=0n,cr=0n; const lines=arr(entry.lines); if(lines.length<2) fail()
    lines.forEach((v,index)=>{const line=rec(v),key=id(line.id); if(lineIds.has(key)||line.line_no!==String(index+1)) fail(); lineIds.add(key); id(line.account_id); const n=reportMoney(line.amount_minor),side=str(line.side); if(n===0n||!['debit','credit'].includes(side)) fail(); if(side==='debit') dr+=n;else cr+=n;rows.push([c(index===0?entryDateTime(date,entry.entry_time):''),c(index===0?optionalText(entry.reference):''),c(index===0?optionalText(entry.explanation):''),a(str(line.account_name)),m(side==='debit'?n:0n),m(side==='credit'?n:0n),c(optionalText(line.memo)),index===0?{text:'檢視分錄',entryId:id(entry.id)}:c('')]) })
    if(dr!==cr||dr!==reportMoney(entry.debit_total_minor)||cr!==reportMoney(entry.credit_total_minor)) fail();totalDr+=dr;totalCr+=cr
  }
  if(totalDr!==reportMoney(r.total_debit_minor)||totalCr!==reportMoney(r.total_credit_minor)||totalDr!==totalCr) fail()
  return {columns:['日期','傳票號碼','摘要','科目','借方','貸方','備註','操作'],rows,summary:[`借方合計 ${fmt(totalDr)}`,`貸方合計 ${fmt(totalCr)}`]}
}
