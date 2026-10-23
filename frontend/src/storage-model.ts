export type StorageAction = 'backup' | 'recipient-request' | 'export' | 'import' | 'container-import' | 'restore' | 'recover-imports' | 'recover-exports'
export type StorageSelectionKind = 'book' | 'request' | 'package' | 'backup' | 'request-destination' | 'export-destination' | 'book-destination' | 'recovery-parent'
export interface StoragePreview { requestId: string; fingerprint: string }
export interface StorageStatus { operationId: string; state: 'running' | 'succeeded' | 'failed'; result: string[] }
export function storageSelectionPrompt(action:StorageAction,again=false):string {
  if(action==='recover-imports')return '請選擇中斷工作所在資料夾，再明確確認復原。'
  if(action==='recover-exports')return '請選擇來源帳簿與中斷工作所在資料夾，再明確確認復原。'
  return again?'請重新選擇每個目的地。':'請選擇來源與全新目的地，再明確確認。'
}
export function backupIdentityStatus(value:unknown):boolean {
  const r=record(value);exact(r,['initialized']);if(typeof r.initialized!=='boolean')return fail();return r.initialized
}
export function backupIdentityInitialized(value:unknown):void {if(!backupIdentityStatus(value))fail()}
export function storageWriteUncertain(error:{status:number|null;code:string}|null):boolean {
  if(error?.status===409&&['backup_identity_missing','backup_busy','storage_busy','storage_capacity'].includes(error.code))return false
  return !error||error.code==='selection_changed'||error.status===409||error.status===null||error.status<400||error.status>=500
}
const fail=(): never=>{throw new Error('儲存作業回應無法安全核對；請保留來源與中斷資料，不要重送。')}
const record=(v:unknown):Record<string,unknown>=>v!==null&&typeof v==='object'&&!Array.isArray(v)?v as Record<string,unknown>:fail()
const exact=(r:Record<string,unknown>,keys:string[]):void=>{if(Object.keys(r).some(k=>!keys.includes(k)))fail()}
export const capabilityId=(v:unknown):string=>typeof v==='string'&&/^[0-9a-f]{32}$/.test(v)?v:fail()
const fingerprint=(v:unknown):string=>typeof v==='string'&&/^[0-9a-f]{64}$/.test(v)?v:fail()
const count=(v:unknown):string=>{
  if(typeof v!=='string'||!/^(0|[1-9]\d*)$/.test(v)||v.length>20||BigInt(v)>18446744073709551615n) return fail()
  return v.replace(/\B(?=(\d{3})+(?!\d))/g,',')
}
export function storagePreview(value:unknown):StoragePreview {const r=record(value);exact(r,['request_id','fingerprint']);return {requestId:capabilityId(r.request_id),fingerprint:fingerprint(r.fingerprint)}}
export function storageResult(value:unknown,action:StorageAction):string[] {
  const r=record(value)
  const fields:Partial<Record<StorageAction,string[]>>={
    'recipient-request':['request_id','fingerprint'],export:['package_id','attachment_count','attachment_bytes'],import:['package_id','attachment_count'],'container-import':['package_id','attachment_count'],restore:['attachment_count','attachment_bytes'],'recover-imports':['removed_count','verified_count','active_count','unrecognized_count'],'recover-exports':['removed_count','verified_count','active_count','unrecognized_count'],
  }
  const required=fields[action];if(!required) return fail()
  exact(r,[...required,'cleanup_warning','selection_required'])
  if(typeof r.cleanup_warning!=='boolean'||typeof r.selection_required!=='boolean'||r.selection_required!==['import','container-import','restore'].includes(action))fail()
  const labels:Record<string,string>={request_id:'收件請求識別',fingerprint:'收件公鑰指紋',package_id:'移轉包識別',attachment_count:'附件數',attachment_bytes:'附件位元組',removed_count:'清理的已驗證工作數',verified_count:'已驗證目的地數',active_count:'仍在進行的工作數',unrecognized_count:'保留的未辨識工作數'}
  const rows=required.map(k=>`${labels[k]}：${k==='fingerprint'?fingerprint(r[k]):k.endsWith('_id')?capabilityId(r[k]):count(r[k])}`)
  if(r.cleanup_warning)rows.push('清理警告：結果已完成，但部分清理未完成；保留中斷資料供檢查。')
  if(r.selection_required)rows.push('新帳簿未自動開啟；請另行明確選擇帳簿。')
  return rows
}
export function storageStatus(value:unknown,action:StorageAction,expectedId?:string):StorageStatus {
  const r=record(value);exact(r,['operation_id','state','result','error_code']);const operationId=capabilityId(r.operation_id)
  if(expectedId!==undefined&&operationId!==expectedId)fail()
  if(r.state==='running') {if('result'in r||'error_code'in r)fail();return {operationId,state:'running',result:[]}}
  if(r.state==='failed') {if('result'in r||r.error_code!=='storage_operation_failed')fail();return {operationId,state:'failed',result:['作業失敗；來源與不確定工作保留，請檢查後使用中斷復原。']}}
  if(r.state==='succeeded') {if('error_code'in r)fail();return {operationId,state:'succeeded',result:storageResult(r.result,action)}}
  return fail()
}
export function backupResult(value:unknown):string[] {
  const r=record(value);exact(r,['backup_name','verified','attachment_count','attachment_bytes','retention_warning','preserved_unrecognized'])
  if(typeof r.backup_name!=='string'||!r.backup_name||r.backup_name.length>255||/[\/\\\x00-\x1f\x7f]/.test(r.backup_name)||r.backup_name==='.'||r.backup_name==='..'||r.verified!==true)fail()
  const rows=[`備份 ${r.backup_name} 已獨立驗證`,`附件數：${count(r.attachment_count)}`,`附件位元組：${count(r.attachment_bytes)}`]
  if(r.retention_warning===null)rows.push(`保留的未辨識項目：${count(r.preserved_unrecognized)}`)
  else {if(typeof r.retention_warning!=='string'||!r.retention_warning||r.preserved_unrecognized!==null)fail();rows.push('備份已成功，但保留／清理作業有警告；原有未辨識內容保持保留。')}
  return rows
}
export const requiredSelections:Record<StorageAction,StorageSelectionKind[]>={backup:[],'recipient-request':['request-destination'],export:['book','request','export-destination'],import:['package','request','book-destination'],'container-import':['request','book-destination'],restore:['backup','book-destination'],'recover-imports':['recovery-parent'],'recover-exports':['book','recovery-parent']}
export function storageCommand(action:StorageAction,selections:Partial<Record<StorageSelectionKind,string>>,preview?:StoragePreview):{path:string;payload:unknown} {
  const get=(kind:StorageSelectionKind):string=>capabilityId(selections[kind]);requiredSelections[action].forEach(get)
  switch(action) {
    case 'backup':return {path:'/api/backups',payload:{}}
    case 'recipient-request':return {path:'/api/storage/recipient-requests',payload:{destination:get('request-destination')}}
    case 'export':if(!preview) return fail();return {path:'/api/storage/exports',payload:{book:get('book'),request:get('request'),destination:get('export-destination'),confirmed_request_id:capabilityId(preview.requestId),confirmed_fingerprint:fingerprint(preview.fingerprint)}}
    case 'import':return {path:'/api/storage/imports',payload:{package:get('package'),request:get('request'),destination:get('book-destination')}}
    case 'restore':return {path:'/api/storage/restores',payload:{backup:get('backup'),destination:get('book-destination')}}
    case 'recover-imports':return {path:'/api/storage/recoveries',payload:{kind:'imports',parent:get('recovery-parent')}}
    case 'recover-exports':return {path:'/api/storage/recoveries',payload:{kind:'exports',book:get('book'),parent:get('recovery-parent')}}
    case 'container-import':return fail()
  }
}
