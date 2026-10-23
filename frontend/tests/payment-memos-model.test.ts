import test from 'node:test'
import assert from 'node:assert/strict'
import { memoFields, paymentDraftAmount, parsePaymentMemo, parsePaymentMemos } from '../src/payment-memos-model.ts'
export const memo={id:'1',revision:'1',paid_amount_minor:'0',remaining_minor:'9007199254740993',payment_draft_id:null,due_date:'2026-10-23',title:'租金',payee:'房東',note:'',amount_minor:'9007199254740993',necessity:'required' as const,status:'planned' as const,created_at:'2026-10-23T00:00:00Z',updated_at:'2026-10-23T00:00:00Z'}
test('planned required total is exact and excludes cancelled and optional',()=>{
 const items=[memo,{...memo,id:'2',necessity:'optional'},{...memo,id:'3',status:'cancelled'}]
 assert.equal(parsePaymentMemos({items,required_total_minor:memo.amount_minor},'2026-10-01','2026-10-31').required,9007199254740993n)
 assert.throws(()=>parsePaymentMemos({items,required_total_minor:'9007199254740992'},'2026-10-01','2026-10-31'))
})
test('memo rejects imprecise and invalid amounts and dates',()=>{
 for(const amount_minor of ['0','01','-1','1.2','1e3','9223372036854775808'])assert.throws(()=>memoFields({...memo,amount_minor}))
 assert.throws(()=>parsePaymentMemo({...memo,amount_minor:9007199254740992}))
 assert.throws(()=>memoFields({...memo,due_date:'2026-02-29'}))
 assert.throws(()=>memoFields({...memo,title:'  '}))
 assert.equal(memoFields({...memo,title:' 租金 '}).title,'租金')
})
test('list rejects duplicate and out of requested range or invalid status/revision',()=>{
 assert.throws(()=>parsePaymentMemos({items:[memo,memo],required_total_minor:'18014398509481986'},'2026-10-01','2026-10-31'))
 assert.throws(()=>parsePaymentMemos({items:[memo],required_total_minor:memo.amount_minor},'2026-10-24','2026-10-31'))
 for(const change of [{revision:'0'},{status:'paid'},{updated_at:'invalid'}])assert.throws(()=>parsePaymentMemo({...memo,...change}))
})
test('remaining and paid reconcile exactly and required total uses remaining',()=>{const item={...memo,paid_amount_minor:'9007199254740992',remaining_minor:'1',payment_draft_id:'7'};assert.equal(parsePaymentMemos({items:[item],required_total_minor:'1'},'2026-10-01','2026-10-31').required,1n);assert.throws(()=>parsePaymentMemo({...item,remaining_minor:'2'}));assert.throws(()=>parsePaymentMemo({...item,payment_draft_id:'01'}))})

test('explicit payment amount is exact positive and bounded by remaining',()=>{
 assert.equal(paymentDraftAmount('9007199254740993','9007199254740994'),'9007199254740993')
 assert.equal(paymentDraftAmount('1','1'),'1')
 for(const value of ['0','-1','01','1.5','1e3','9007199254740995','9223372036854775808'])assert.throws(()=>paymentDraftAmount(value,'9007199254740994'))
 assert.throws(()=>paymentDraftAmount('1','0'))
})
