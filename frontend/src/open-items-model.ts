import { isPositiveI64Decimal } from './account-tree.ts'
import { reportDate, reportMoney } from './reports-model.ts'
export interface OpenItem {id:string;source_line_id:string;source_entry_id:string;account_id:string;kind:'receivable'|'payable';entry_date:string;amount_minor:string;description:string;counterparty:string|null;due_date:string|null;paid_minor:string;adjusted_minor:string;outstanding_minor:string;reversed:boolean}
export interface Allocation {id:string;settlement_line_id:string;settlement_entry_id:string;item_id:string;entry_date:string;amount_minor:string;kind:'payment'|'adjustment';created_at:string;void_reason:string|null;voided_at:string|null}
const validId=(value:unknown):boolean=>typeof value==='string'&&isPositiveI64Decimal(value)
const fail=():never=>{throw new Error('應收／應付資料無法安全核對')}
export function parseOpenItems(raw:unknown):{items:OpenItem[];allocations:Allocation[]} {
  if(!raw||typeof raw!=='object')return fail()
  const r=raw as Record<string,unknown>
  if(r.coverage!=='explicit-lines-only'||!Array.isArray(r.items)||!Array.isArray(r.allocations))return fail()
  const items=r.items as OpenItem[],allocations=r.allocations as Allocation[],ids=new Set<string>(),aids=new Set<string>()
  for(const i of items){if(!i||!['id','source_line_id','source_entry_id','account_id'].every(k=>validId((i as unknown as Record<string,unknown>)[k]))||ids.has(i.id)||!['receivable','payable'].includes(i.kind)||!reportDate(i.entry_date)||typeof i.description!=='string'||typeof i.reversed!=='boolean'||!(i.counterparty===null||typeof i.counterparty==='string')||!(i.due_date===null||reportDate(i.due_date)))fail();ids.add(i.id);const amount=reportMoney(i.amount_minor),paid=reportMoney(i.paid_minor),adjusted=reportMoney(i.adjusted_minor),outstanding=reportMoney(i.outstanding_minor);if((i.reversed&&(paid!==0n||adjusted!==0n))||amount<=0n||paid<0n||adjusted<0n||outstanding<0n||paid+adjusted>amount||outstanding!==(i.reversed?0n:amount-paid-adjusted))fail()}
  for(const a of allocations){if(!a||!['id','settlement_line_id','settlement_entry_id','item_id'].every(k=>validId((a as unknown as Record<string,unknown>)[k]))||aids.has(a.id)||!ids.has(a.item_id)||!reportDate(a.entry_date)||reportMoney(a.amount_minor)<=0n||!['payment','adjustment'].includes(a.kind)||typeof a.created_at!=='string'||!(a.void_reason===null||typeof a.void_reason==='string')||!(a.voided_at===null||typeof a.voided_at==='string')||(a.void_reason===null)!==(a.voided_at===null))fail();aids.add(a.id)}
  for(const i of items){let p=0n,d=0n;for(const a of allocations.filter(a=>a.item_id===i.id&&a.voided_at===null)){if(a.kind==='payment')p+=BigInt(a.amount_minor);else d+=BigInt(a.amount_minor)}if(p!==BigInt(i.paid_minor)||d!==BigInt(i.adjusted_minor))fail()}
  return {items,allocations}
}
export function exactPositive(value:string):string {if(!isPositiveI64Decimal(value))fail();return value}
export interface SplitInput {amount_minor:string;description:string;counterparty:string|null;due_date:string|null}
export function validateSplit(sourceAmount:string,rows:SplitInput[]):SplitInput[]{
  const expected=BigInt(exactPositive(sourceAmount));let total=0n
  if(rows.length>100)throw new Error('每行最多拆分 100 筆款項')
  if(rows.length===0)throw new Error('至少需要一筆款項')
  for(const row of rows){total+=BigInt(exactPositive(row.amount_minor));if(!row.description.trim()||(row.due_date!==null&&!reportDate(row.due_date)))throw new Error('請填寫每筆事項與有效到期日')}
  if(total!==expected)throw new Error('拆分合計必須等於所選來源行金額')
  return rows.map(row=>({...row,description:row.description.trim(),counterparty:row.counterparty?.trim()||null}))
}
export interface OpenItemFilter {kind:'all'|'receivable'|'payable';status:'all'|'outstanding'|'closed'|'reversed';keyword:string}
export function filterOpenItems(items:OpenItem[],filter:OpenItemFilter):OpenItem[]{const query=filter.keyword.trim().toLocaleLowerCase();return items.filter(i=>(filter.kind==='all'||i.kind===filter.kind)&&(filter.status==='all'||(filter.status==='reversed'?i.reversed:!i.reversed&&(filter.status==='outstanding'?BigInt(i.outstanding_minor)>0n:BigInt(i.outstanding_minor)===0n)))&&(!query||`${i.description} ${i.counterparty??''}`.toLocaleLowerCase().includes(query)))}
