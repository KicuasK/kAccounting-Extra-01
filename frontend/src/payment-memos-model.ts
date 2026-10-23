import { isPositiveI64Decimal } from './account-tree.ts'
import { reportDate, reportMoney } from './reports-model.ts'
export interface PaymentMemoFields { due_date:string; title:string; payee:string; note:string; amount_minor:string; necessity:'required'|'optional' }
export interface PaymentMemo extends PaymentMemoFields { id:string; revision:string; status:'planned'|'cancelled'; created_at:string; updated_at:string; paid_amount_minor:string; remaining_minor:string; payment_draft_id:string|null }
const fail=():never=>{throw new Error('付款備忘錄資料無法安全核對')}
export function memoFields(value: PaymentMemoFields): PaymentMemoFields {
  if(!reportDate(value.due_date)||!value.title.trim()||[...value.title].length>200||[...value.payee].length>200||[...value.note].length>2000||/[\u0000-\u001f\u007f-\u009f]/u.test(value.title+value.payee)||/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(value.note)||!['required','optional'].includes(value.necessity)||reportMoney(value.amount_minor)<=0n) fail()
  return {...value,title:value.title.trim(),payee:value.payee.trim(),note:value.note.trim()}
}
export function parsePaymentMemo(value:unknown):PaymentMemo {
  if(!value||typeof value!=='object'||Array.isArray(value))return fail()
  const r=value as Record<string,unknown>
  for(const key of ['id','revision','due_date','title','payee','note','amount_minor','necessity','status','created_at','updated_at'])if(typeof r[key]!=='string')fail()
  const memo=r as unknown as PaymentMemo
  if(!isPositiveI64Decimal(memo.id)||!isPositiveI64Decimal(memo.revision)||!['planned','cancelled'].includes(memo.status)||Number.isNaN(Date.parse(memo.created_at))||Number.isNaN(Date.parse(memo.updated_at)))fail()
  memoFields(memo);const paid=reportMoney(memo.paid_amount_minor),remaining=reportMoney(memo.remaining_minor);if(paid+remaining!==BigInt(memo.amount_minor)||(memo.payment_draft_id!==null&&(typeof memo.payment_draft_id!=='string'||!isPositiveI64Decimal(memo.payment_draft_id))))fail();return {...memo}
}
export function parsePaymentMemos(value:unknown,from:string,through:string):{items:PaymentMemo[];required:bigint} {
  if(!reportDate(from)||!reportDate(through)||from>through||!value||typeof value!=='object')return fail()
  const r=value as Record<string,unknown>;if(!Array.isArray(r.items))return fail()
  const seen=new Set<string>(),items=r.items.map(parsePaymentMemo);let required=0n
  for(const item of items){if(seen.has(item.id)||item.due_date<from||item.due_date>through)fail();seen.add(item.id);if(item.status==='planned'&&item.necessity==='required')required+=BigInt(item.remaining_minor)}
  if(reportMoney(r.required_total_minor)!==required)fail()
  return {items,required}
}

/** Validate an explicit draft amount without converting monetary values to Number. */
export function paymentDraftAmount(value:string,remaining:string):string {
  const amount=reportMoney(value),available=reportMoney(remaining)
  if(amount<=0n||amount>available) return fail()
  return value
}
