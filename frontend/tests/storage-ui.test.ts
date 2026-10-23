import test from 'node:test'
import assert from 'node:assert/strict'
import { mountStorageUI, type StorageContext } from '../src/storage-ui.ts'
import { ApiError } from '../src/session-client.ts'
class Node {
  tag:string;value='';type='';disabled=false;checked=false;className='';private own='';children:Node[]=[];listeners=new Map<string,(()=>void)[]>()
  constructor(tag:string){this.tag=tag}
  set textContent(v:string){this.own=v;this.children=[]}
  get textContent():string{return this.own+this.children.map(n=>n.textContent).join('')}
  append(...nodes:Node[]):void{this.children.push(...nodes)}
  prepend(...nodes:Node[]):void{this.children.unshift(...nodes)}
  replaceChildren(...nodes:Node[]):void{this.own='';this.children=[...nodes]}
  remove():void{}
  setAttribute():void{}
  addEventListener(key:string,callback:()=>void):void{this.listeners.set(key,[...this.listeners.get(key)??[],callback])}
  all():Node[]{return this.children.flatMap(n=>[n,...n.all()])}
  fire(key:string):void{if(!this.disabled)this.listeners.get(key)?.forEach(fn=>fn())}
}
const id='a'.repeat(32),hash='b'.repeat(64),selection={bookId:'test',selectionId:'c'.repeat(32)}
function button(host:Node,text:string):Node{const n=host.all().find(n=>n.tag==='button'&&n.textContent===text);assert.ok(n,text);return n}
async function settle():Promise<void>{for(let i=0;i<30;i++)await Promise.resolve()}
function setup(overrides:Partial<StorageContext>={}){
  Object.defineProperty(globalThis,'document',{configurable:true,value:{createElement:(tag:string)=>new Node(tag)}})
  Object.defineProperty(globalThis,'window',{configurable:true,value:{confirm:()=>true}})
  const host=new Node('host'),calls:string[]=[]
  const ui=mountStorageUI(host as unknown as HTMLElement,{getSelection:()=>selection,onSelectionChanged(){},async request(path){calls.push(path);return {backup_name:'verified-backup',verified:true,attachment_count:'0',attachment_bytes:'0',retention_warning:null,preserved_unrecognized:'0'}},...overrides})
  return {host,calls,ui}
}
function confirm(host:Node,label:string):void{button(host,'檢查並確認此作業').fire('click');const check=host.all().find(n=>n.type==='checkbox');assert.ok(check);assert.equal(button(host,label).disabled,true);check.checked=true;check.fire('change');button(host,label).fire('click')}
test('backup performs no write without explicit checkbox confirmation and preserves warning success',async()=>{
  const {host,calls,ui}=setup();assert.equal(calls.length,0)
  confirm(host,'確認建立加密備份');await settle();assert.deepEqual(calls,['/api/backups']);assert.match(host.textContent,/已獨立驗證/);assert.equal(ui.hasUnsavedChanges(),false);ui.dispose()
})
test('unknown submission never retries and explicit acknowledgement only allows leaving',async()=>{
  let writes=0;const {host,ui}=setup({async request(){writes++;throw new Error('network')}})
  confirm(host,'確認建立加密備份');await settle();assert.equal(writes,1);assert.equal(ui.hasUnsavedChanges(),true)
  assert.equal(button(host,'檢查並確認此作業').disabled,true)
  button(host,'已記下待核對狀態，允許離開此畫面').fire('click');assert.equal(ui.hasUnsavedChanges(),false)
  assert.equal(button(host,'檢查並確認此作業').disabled,true);assert.equal(writes,1);ui.dispose()
})
test('definite backup identity refusal is actionable without uncertain lock or automatic retry',async()=>{
  let writes=0;const {host,ui}=setup({async request(){writes++;throw new ApiError('backup_identity_missing',409)}})
  confirm(host,'確認建立加密備份');await settle();assert.equal(writes,1);assert.equal(button(host,'檢查並確認此作業').disabled,true);assert.equal(button(host,'設定此帳簿的備份身分').disabled,false);assert.equal(ui.hasUnsavedChanges(),false);ui.dispose()
})
test('export requires typed native selections fingerprint confirmation and verified job before download',async()=>{
  const kinds:string[]=[],calls:string[]=[],downloads:string[]=[]
  const {host,ui}=setup({async selectStorage(kind){kinds.push(kind);return {capabilityId:id}},async request(path,_signal,method){calls.push(`${method??'GET'} ${path}`);if(path.endsWith('/request'))return {request_id:id,fingerprint:hash};if(path.endsWith('/exports'))return {operation_id:id,state:'running'};return {operation_id:id,state:'succeeded',result:{package_id:id,attachment_count:'0',attachment_bytes:'0',cleanup_warning:false,selection_required:false}}},async downloadStorage(id){downloads.push(id)}})
  const select=host.all().find(n=>n.tag==='select')!;select.value='export';select.fire('change')
  for(const name of ['來源帳簿','收件請求檔','新的匯出資料夾']){button(host,`${name}：選擇…`).fire('click');await settle()}
  assert.deepEqual(kinds,['book','request','export-destination']);assert.match(host.textContent,new RegExp(hash));assert.equal(calls.filter(c=>c.startsWith('POST')).length,0)
  confirm(host,'確認匯出給另一台 Mac');await settle();assert.equal(ui.hasUnsavedChanges(),true)
  button(host,'重新查詢進度').fire('click');await settle();assert.equal(ui.hasUnsavedChanges(),false)
  button(host,'交給 Safari 下載至 Downloads').fire('click');await settle();assert.deepEqual(downloads,[id]);assert.match(host.textContent,/Downloads 確認完成/);ui.dispose()
})
test('cancelled native selection does not submit or retain a destination',async()=>{
  const {host,calls,ui}=setup({async selectStorage(){return null}})
  const select=host.all().find(n=>n.tag==='select')!;select.value='restore';select.fire('change')
  assert.match(host.textContent,/不選外層 managed 資料夾/)
  button(host,'加密備份包（backup.kabackup）：選擇…').fire('click');await settle();assert.equal(calls.length,0);assert.equal(ui.hasUnsavedChanges(),false);assert.equal(button(host,'檢查並確認此作業').disabled,true);ui.dispose()
})
test('unknown import admission can explicitly enter recovery without resubmitting the import',async()=>{
  const calls:string[]=[];const {host,ui}=setup({async selectStorage(){return {capabilityId:id}},async request(path){calls.push(path);throw new Error('unknown admission')}})
  const select=host.all().find(n=>n.tag==='select')!;select.value='import';select.fire('change')
  for(const name of ['移轉包資料夾','收件請求檔','新的帳簿目的地']){button(host,`${name}：選擇…`).fire('click');await settle()}
  confirm(host,'確認匯入資料夾移轉包');await settle();assert.deepEqual(calls,['/api/storage/imports']);assert.equal(ui.hasUnsavedChanges(),true)
  button(host,'確認轉至中斷復原（不重送原作業）').fire('click')
  assert.equal(select.value,'recover-imports');assert.equal(calls.length,1)
  assert.equal(button(host,'中斷工作所在上層資料夾：選擇…').disabled,false)
  assert.equal(button(host,'檢查並確認此作業').disabled,true);ui.dispose()
})
test('poll result for another job never enables a download or displays false success',async()=>{
  let submitted=false;const {host,ui}=setup({async selectStorage(){return {capabilityId:id}},async request(path){if(path.endsWith('/request'))return {request_id:id,fingerprint:hash};if(path.endsWith('/exports')){submitted=true;return {operation_id:id,state:'running'}}return {operation_id:'d'.repeat(32),state:'succeeded',result:{package_id:id,attachment_count:'0',attachment_bytes:'0',cleanup_warning:false,selection_required:false}}}})
  const select=host.all().find(n=>n.tag==='select')!;select.value='export';select.fire('change')
  for(const name of ['來源帳簿','收件請求檔','新的匯出資料夾']){button(host,`${name}：選擇…`).fire('click');await settle()}
  confirm(host,'確認匯出給另一台 Mac');await settle();assert.equal(submitted,true)
  button(host,'重新查詢進度').fire('click');await settle()
  assert.equal(host.all().some(n=>n.tag==='button'&&n.textContent==='交給 Safari 下載至 Downloads'),false)
  assert.equal(ui.hasUnsavedChanges(),true);assert.match(host.textContent,/無法核對作業結果/);ui.dispose()
})
function confirmIdentity(host:Node):void{button(host,'設定此帳簿的備份身分').fire('click');const check=host.all().find(n=>n.type==='checkbox');assert.ok(check);assert.equal(button(host,'確認建立備份身分').disabled,true);check.checked=true;check.fire('change');button(host,'確認建立備份身分').fire('click')}
const backup={backup_name:'new-namespace-backup',verified:true,attachment_count:'0',attachment_bytes:'0',retention_warning:null,preserved_unrecognized:'0'}
test('missing identity only exposes separate explicit setup; successful initialization never auto-backs-up',async()=>{
  const calls:string[]=[];let initialized=false
  const {host,ui}=setup({async request(path,_s,method='GET',payload){calls.push(`${method} ${path}`);if(path==='/api/backups/identity'){assert.deepEqual(payload,{});initialized=true;return {initialized:true}}if(!initialized)throw new ApiError('backup_identity_missing',409);return backup}})
  assert.equal(host.all().some(n=>n.textContent==='設定此帳簿的備份身分'),false)
  confirm(host,'確認建立加密備份');await settle();assert.deepEqual(calls,['POST /api/backups']);assert.match(host.textContent,/舊備份與原有加密金鑰都會保留/);assert.doesNotMatch(host.textContent,/命名空間|清理權限/)
  confirmIdentity(host);await settle();assert.deepEqual(calls,['POST /api/backups','POST /api/backups/identity']);assert.equal(ui.hasUnsavedChanges(),false)
  confirm(host,'確認建立加密備份');await settle();assert.deepEqual(calls,['POST /api/backups','POST /api/backups/identity','POST /api/backups']);assert.match(host.textContent,/已獨立驗證/);ui.dispose()
})
test('unknown identity initialization locks repeat POST until explicit successful status reconciliation',async()=>{
  const calls:string[]=[];const {host,ui}=setup({async request(path,_s,method='GET'){calls.push(`${method} ${path}`);if(path==='/api/backups')throw new ApiError('backup_identity_missing',409);if(method==='POST')throw new Error('connection lost');return {initialized:true}}})
  confirm(host,'確認建立加密備份');await settle();confirmIdentity(host);await settle()
  assert.equal(ui.hasUnsavedChanges(),true);assert.equal(button(host,'檢查並確認此作業').disabled,true)
  assert.equal(host.all().some(n=>n.tag==='button'&&n.textContent==='設定此帳簿的備份身分'),false)
  button(host,'核對備份身分狀態').fire('click');await settle();assert.deepEqual(calls,['POST /api/backups','POST /api/backups/identity','GET /api/backups/identity']);assert.equal(ui.hasUnsavedChanges(),false);assert.equal(button(host,'檢查並確認此作業').disabled,false);ui.dispose()
})
test('missing status permits only a new explicit identity confirmation, malformed status remains locked',async()=>{
  let response:unknown={initialized:'false'};let posts=0
  const {host,ui}=setup({async request(path,_s,method='GET'){if(path==='/api/backups')throw new ApiError('backup_identity_missing',409);if(method==='POST'){posts++;throw new Error('unknown')}return response}})
  confirm(host,'確認建立加密備份');await settle();confirmIdentity(host);await settle();button(host,'核對備份身分狀態').fire('click');await settle();assert.equal(ui.hasUnsavedChanges(),true);assert.equal(posts,1)
  response={initialized:false};button(host,'核對備份身分狀態').fire('click');await settle();assert.equal(button(host,'設定此帳簿的備份身分').disabled,false);assert.equal(posts,1);assert.equal(button(host,'檢查並確認此作業').disabled,true);ui.dispose()
})
test('identity exists conflict requires read reconciliation instead of replacement or automatic backup',async()=>{
  const calls:string[]=[];const {host,ui}=setup({async request(path,_s,method='GET'){calls.push(`${method} ${path}`);if(path==='/api/backups')throw new ApiError('backup_identity_missing',409);if(method==='POST')throw new ApiError('backup_identity_exists',409);return {initialized:true}}})
  confirm(host,'確認建立加密備份');await settle();confirmIdentity(host);await settle();assert.equal(ui.hasUnsavedChanges(),true);button(host,'核對備份身分狀態').fire('click');await settle();assert.equal(calls.length,3);assert.equal(ui.hasUnsavedChanges(),false);ui.dispose()
})
test('lost operation ID after restart exposes explicit recovery and never resubmits original import',async()=>{
  const calls:string[]=[];const {host,ui}=setup({async selectStorage(){return {capabilityId:id}},async request(path){calls.push(path);if(path==='/api/storage/imports')return {operation_id:id,state:'running'};throw new ApiError('storage_operation_not_found',404)}})
  const select=host.all().find(n=>n.tag==='select')!;select.value='import';select.fire('change');for(const name of ['移轉包資料夾','收件請求檔','新的帳簿目的地']){button(host,`${name}：選擇…`).fire('click');await settle()}
  confirm(host,'確認匯入資料夾移轉包');await settle();button(host,'重新查詢進度').fire('click');await settle();assert.match(host.textContent,/已無原工作識別/)
  button(host,'確認轉至中斷復原（不重送原作業）').fire('click');assert.equal(select.value,'recover-imports');assert.equal(calls.filter(p=>p==='/api/storage/imports').length,1);assert.equal(ui.hasUnsavedChanges(),false);ui.dispose()
})
test('failed recovery never reports success or resubmits; successful recovery preserves unknown work counts',async()=>{
  let succeeded=false;const calls:string[]=[];const {host,ui}=setup({async selectStorage(){return {capabilityId:id}},async request(path){calls.push(path);if(path==='/api/storage/recoveries')return {operation_id:id,state:'running'};return succeeded?{operation_id:id,state:'succeeded',result:{removed_count:'0',verified_count:'1',active_count:'0',unrecognized_count:'2',cleanup_warning:false,selection_required:false}}:{operation_id:id,state:'failed',error_code:'storage_operation_failed'}}})
  const select=host.all().find(n=>n.tag==='select')!;select.value='recover-imports';select.fire('change');assert.match(host.textContent,/請選擇中斷工作所在資料夾，再明確確認復原/);assert.doesNotMatch(host.textContent,/全新目的地/);button(host,'中斷工作所在上層資料夾：選擇…').fire('click');await settle();confirm(host,'確認復原中斷匯入');await settle();button(host,'重新查詢進度').fire('click');await settle()
  assert.match(host.textContent,/作業失敗/);assert.doesNotMatch(host.textContent,/已驗證目的地數/);assert.equal(calls.filter(p=>p==='/api/storage/recoveries').length,1)
  button(host,'開始另一項作業').fire('click');assert.match(host.textContent,/請選擇中斷工作所在資料夾，再明確確認復原/);assert.doesNotMatch(host.textContent,/請重新選擇每個目的地/);succeeded=true;button(host,'中斷工作所在上層資料夾：選擇…').fire('click');await settle();confirm(host,'確認復原中斷匯入');await settle();button(host,'重新查詢進度').fire('click');await settle()
  assert.match(host.textContent,/保留的未辨識工作數：2/);assert.equal(calls.filter(p=>p==='/api/storage/recoveries').length,2);assert.equal(ui.hasUnsavedChanges(),false);ui.dispose()
})
test('unknown recovery admission never repeats POST or invents completed recovery results',async()=>{
  let writes=0;const {host,ui}=setup({async selectStorage(){return {capabilityId:id}},async request(){writes++;throw new Error('lost')}})
  const select=host.all().find(n=>n.tag==='select')!;select.value='recover-exports';select.fire('change')
  for(const name of ['來源帳簿','中斷工作所在上層資料夾']){button(host,`${name}：選擇…`).fire('click');await settle()}
  confirm(host,'確認復原中斷匯出');await settle();assert.equal(writes,1);assert.equal(button(host,'檢查並確認此作業').disabled,true);assert.doesNotMatch(host.textContent,/已驗證目的地數/)
  button(host,'已記下待核對狀態，允許離開此畫面').fire('click');assert.equal(ui.hasUnsavedChanges(),false);assert.equal(writes,1);ui.dispose()
})
