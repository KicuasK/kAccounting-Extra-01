import test from 'node:test'
import assert from 'node:assert/strict'
import { parseReport, reportMoney, reportDate, reportAccountOptions } from '../src/reports-model.ts'
import { parseAccountTree } from '../src/account-tree.ts'
const query={from:'2026-01-01',through:'2026-10-23'}
const row=(account_id:string,element:string,d:string,c:string)=>({account_id,account_name:'科目',element,debit_turnover_minor:d,credit_turnover_minor:c,debit_balance_minor:d,credit_balance_minor:c})
test('trial balance preserves values above Number safe integer and reconciles exact sums',()=>{
  const n='9007199254740993',report={as_of:query.through,rows:[row('1','asset',n,'0'),row('2','equity','0',n)],total_debit_minor:n,total_credit_minor:n}
  assert.match(parseReport('trial-balance',report,query).summary[0]!,/9,007,199,254,740,993/)
  assert.throws(()=>parseReport('trial-balance',{...report,total_credit_minor:'9007199254740992'},query))
  assert.throws(()=>parseReport('trial-balance',{...report,as_of:'2026-10-22'},query))
  assert.throws(()=>parseReport('trial-balance',{...report,rows:[report.rows[0],report.rows[0]]},query))
})
test('strict decimal strings and real Gregorian dates',()=>{
  for(const value of [1,1.5,'01','-0','1e3','9223372036854775808']) assert.throws(()=>reportMoney(value))
  assert.equal(reportMoney('-9223372036854775808',true),-9223372036854775808n)
  assert.equal(reportDate('2024-02-29'),true);assert.equal(reportDate('2026-02-29'),false)
})
test('income expense normal direction activity is explicit and reconciled',()=>{
  const report={...query,rows:[{account_id:'1',account_name:'收益',element:'income',debit_turnover_minor:'20',credit_turnover_minor:'10',net_activity_minor:'-10'}],total_income_minor:'-10',total_expense_minor:'0',net_result_minor:'-10'}
  assert.match(parseReport('income-expense',report,query).summary[2]!,/-10/)
  assert.equal(parseReport('income-expense',report,query).rows[0]![4]!.text,'借 10')
  assert.throws(()=>parseReport('income-expense',{...report,net_result_minor:'10'},query))
})
test('general journal checks posted date, line ordering and atomic balance',()=>{
  const line={id:'1',line_no:'1',account_id:'1',account_name:'現金',side:'debit',amount_minor:'8',memo:null}
  const entry={id:'1',entry_date:query.from,posted_at:'2026-01-01T00:00:00Z',reference:null,explanation:null,debit_total_minor:'8',credit_total_minor:'8',lines:[line,{...line,id:'2',line_no:'2',side:'credit'}]}
  const report={...query,entries:[entry],total_debit_minor:'8',total_credit_minor:'8'}
  assert.equal(parseReport('general-journal',report,query).rows.length,2)
  assert.equal(parseReport('general-journal',report,query).rows[0]![7]!.entryId,'1')
  assert.throws(()=>parseReport('general-journal',{...report,entries:[{...entry,entry_date:'2025-12-31'}]},query))
  assert.throws(()=>parseReport('general-journal',{...report,entries:[{...entry,lines:[line,{...line,id:'2',line_no:'2'}]}]},query))
})
test('ledger verifies each running balance and requested account',()=>{
  const report={...query,account_id:'1',account_name:'現金',element:'asset',opening_debit_balance_minor:'9',opening_credit_balance_minor:'0',lines:[{entry_id:'1',line_id:'1',line_no:'1',entry_date:query.from,side:'credit',amount_minor:'10',debit_balance_minor:'0',credit_balance_minor:'1',reference:'',explanation:'',memo:''}],period_debit_turnover_minor:'0',period_credit_turnover_minor:'10',closing_debit_balance_minor:'0',closing_credit_balance_minor:'1'}
  assert.equal(parseReport('ledger',report,{...query,accountId:'1'}).rows.length,2)
  assert.throws(()=>parseReport('ledger',report,{...query,accountId:'2'}))
  assert.throws(()=>parseReport('ledger',{...report,closing_credit_balance_minor:'2'},{...query,accountId:'1'}))
})
test('balance sheet fails closed on missing or cyclic hierarchy and unmatched equation',()=>{
  const report={as_of:query.through,hierarchy:[],rows:[],total_assets_minor:'0',total_liabilities_minor:'0',total_equity_minor:'0',unclosed_net_result_minor:'0',total_liabilities_and_equity_minor:'0'}
  assert.equal(parseReport('balance-sheet',report,query).rows.length,0)
  assert.throws(()=>parseReport('balance-sheet',{...report,hierarchy:undefined},query))
  assert.throws(()=>parseReport('balance-sheet',{...report,unclosed_net_result_minor:'1',total_liabilities_and_equity_minor:'1'},query))
  assert.throws(()=>parseReport('balance-sheet',{...report,hierarchy:[{id:'1',parent_id:'1'}]},query))
})
test('balance sheet preserves approved hierarchy and separates unclosed net income',()=>{
  const account=(id:string,name:string,element:string,parent_id:string|null=null,kind='posting')=>({id,name,element,parent_id,kind,is_active:true,revision:'1',created_at:'2026-01-01T00:00:00Z',updated_at:'2026-01-01T00:00:00Z'})
  const report={as_of:query.through,hierarchy:[account('1','資產','asset',null,'category'),account('2','現金','asset','1'),account('3','權益','equity')],rows:[{account_id:'2',account_name:'現金',element:'asset',balance_minor:'10'},{account_id:'3',account_name:'權益',element:'equity',balance_minor:'7'}],total_assets_minor:'10',total_liabilities_minor:'0',total_equity_minor:'7',unclosed_net_result_minor:'3',total_liabilities_and_equity_minor:'10'}
  const table=parseReport('balance-sheet',report,query)
  assert.equal(table.rows[1]![0]!.text,'　現金')
  assert.equal(table.rows[1]![2]!.text,'借 10')
  assert.equal(table.rows[2]![2]!.text,'貸 7')
  assert.ok(table.summary.includes('未結轉淨損益 3'))
  assert.throws(()=>parseReport('balance-sheet',{...report,rows:[{...report.rows[0],account_name:'其他'},report.rows[1]]},query))
  const noRows={...report,hierarchy:[account('2','現金','asset')],rows:[],total_assets_minor:'0',total_equity_minor:'0',unclosed_net_result_minor:'0',total_liabilities_and_equity_minor:'0'}
  assert.equal(parseReport('balance-sheet',noRows,query).rows[0]![2]!.text,'無已過帳資料')
  assert.throws(()=>parseReport('balance-sheet',{...report,rows:[report.rows[0],report.rows[0]]},query))
  assert.throws(()=>parseReport('balance-sheet',{...report,total_assets_minor:10},query))
  assert.throws(()=>parseReport('balance-sheet',{...report,rows:[{...report.rows[0],balance_minor:undefined},report.rows[1]]},query))
  assert.throws(()=>parseReport('balance-sheet',{...report,as_of:'2026-02-30'},query))
})
test('trial and income rows require strictly increasing exact account IDs',()=>{
  const first='9007199254740992',second='9007199254740993'
  const report={as_of:query.through,rows:[row(first,'asset','5','0'),row(second,'equity','0','5')],total_debit_minor:'5',total_credit_minor:'5'}
  assert.equal(parseReport('trial-balance',report,query).rows.length,2)
  assert.throws(()=>parseReport('trial-balance',{...report,rows:[...report.rows].reverse()},query))
  const income={...query,rows:[{account_id:first,account_name:'收益',element:'income',debit_turnover_minor:'0',credit_turnover_minor:'1',net_activity_minor:'1'},{account_id:second,account_name:'費損',element:'expense',debit_turnover_minor:'1',credit_turnover_minor:'0',net_activity_minor:'1'}],total_income_minor:'1',total_expense_minor:'1',net_result_minor:'0'}
  assert.equal(parseReport('income-expense',income,query).rows[0]![4]!.text,'貸 1')
  assert.throws(()=>parseReport('income-expense',{...income,rows:[...income.rows].reverse()},query))
})
test('general journal rejects same-date shuffled and duplicate entry identities',()=>{
  function entry(id:string,line:string) {return {id,entry_date:query.from,posted_at:'2026-01-01T00:00:00Z',reference:null,explanation:null,debit_total_minor:'1',credit_total_minor:'1',lines:[{id:line,line_no:'1',account_id:'1',account_name:'A',side:'debit',amount_minor:'1',memo:null},{id:String(BigInt(line)+1n),line_no:'2',account_id:'2',account_name:'B',side:'credit',amount_minor:'1',memo:null}]}}
  const entries=[entry('9007199254740992','1'),entry('9007199254740993','3')],report={...query,entries,total_debit_minor:'2',total_credit_minor:'2'}
  assert.equal(parseReport('general-journal',report,query).rows.length,4)
  assert.throws(()=>parseReport('general-journal',{...report,entries:[...entries].reverse()},query))
  assert.throws(()=>parseReport('general-journal',{...report,entries:[entries[0],entries[0]]},query))
})
test('ledger checks exact date entry line-number line-ID ordering independently of sums',()=>{
  const line=(entry_id:string,line_no:string,line_id:string,balance:string)=>({entry_id,line_no,line_id,entry_date:query.from,side:'debit',amount_minor:'1',debit_balance_minor:balance,credit_balance_minor:'0',reference:null,explanation:null,memo:null})
  const report={...query,account_id:'1',account_name:'A',element:'asset',opening_debit_balance_minor:'0',opening_credit_balance_minor:'0',lines:[line('9007199254740992','1','1','1'),line('9007199254740993','1','2','2')],period_debit_turnover_minor:'2',period_credit_turnover_minor:'0',closing_debit_balance_minor:'2',closing_credit_balance_minor:'0'}
  const q={...query,accountId:'1'}
  assert.equal(parseReport('ledger',report,q).rows.length,3)
  assert.throws(()=>parseReport('ledger',{...report,lines:[line('9007199254740993','1','1','1'),line('9007199254740992','1','2','2')]},q))
  assert.throws(()=>parseReport('ledger',{...report,lines:[line('1','2','1','1'),line('1','1','2','2')]},q))
  assert.throws(()=>parseReport('ledger',{...report,lines:[line('1','1','2','1'),line('1','1','1','2')]},q))
  assert.throws(()=>parseReport('ledger',{...report,lines:[line('1','1','1','1'),line('1','2','1','2')]},q))
})
test('ledger account choices distinguish equal names by element and full path',()=>{
  const account=(id:string,name:string,element:string,parent_id:string|null,kind:string)=>({id,name,element,parent_id,kind,is_active:false,revision:'1',created_at:'2026-01-01',updated_at:'2026-01-01'})
  const tree=parseAccountTree([{account:account('1','銀行','asset',null,'category'),children:[{account:account('2','現金','asset','1','posting'),children:[]}]},{account:account('3','現金','liability',null,'posting'),children:[]}])
  assert.deepEqual(reportAccountOptions(tree),[{id:'2',label:'資產 · 銀行 › 現金'},{id:'3',label:'負債 · 現金'}])
})

test('journal entry time is displayed without replacing accounting date filters', () => {
  const line = { id: '1', line_no: '1', account_id: '1', account_name: '現金', side: 'debit', amount_minor: '8', memo: null }
  const entry = { id: '1', entry_date: query.from, posted_at: '2026-01-01T00:00:00Z', reference: null, explanation: null, debit_total_minor: '8', credit_total_minor: '8', lines: [line, { ...line, id: '2', line_no: '2', side: 'credit' }] }
  const report = { ...query, entries: [entry], total_debit_minor: '8', total_credit_minor: '8' }
  for (const time of [undefined, null, '00:00:00', '23:59:59']) {
    const result = parseReport('general-journal', { ...report, entries: [{ ...entry, entry_time: time }] }, query)
    assert.equal(result.rows[0]![0]!.text, query.from + (time ? ` ${time}` : ''))
    assert.equal(result.rows[1]![0]!.text, '')
  }
  for (const time of ['', '12:30', '24:00:00', 0]) assert.throws(() => parseReport('general-journal', { ...report, entries: [{ ...entry, entry_time: time }] }, query))
  assert.throws(() => parseReport('general-journal', { ...report, entries: [{ ...entry, entry_date: '2025-12-31', entry_time: '23:59:59' }] }, query))
})

test('ledger displays optional time and rejects malformed time without changing exact running balances', () => {
  const line = { entry_id: '1', line_id: '1', line_no: '1', entry_date: query.from, side: 'credit', amount_minor: '10', debit_balance_minor: '0', credit_balance_minor: '1', reference: '', explanation: '', memo: '' }
  const report = { ...query, account_id: '1', account_name: '現金', element: 'asset', opening_debit_balance_minor: '9', opening_credit_balance_minor: '0', lines: [line], period_debit_turnover_minor: '0', period_credit_turnover_minor: '10', closing_debit_balance_minor: '0', closing_credit_balance_minor: '1' }
  const q = { ...query, accountId: '1' }
  for (const time of [undefined, null, '09:08:07']) assert.equal(parseReport('ledger', { ...report, lines: [{ ...line, entry_time: time }] }, q).rows[1]![0]!.text, query.from + (time ? ` ${time}` : ''))
  assert.throws(() => parseReport('ledger', { ...report, lines: [{ ...line, entry_time: '09:60:00' }] }, q))
})
