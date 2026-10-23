import test from 'node:test'
import assert from 'node:assert/strict'
import { backupIdentityInitialized, backupIdentityStatus, backupResult, capabilityId, requiredSelections, storageCommand, storagePreview, storageResult, storageStatus, storageWriteUncertain } from '../src/storage-model.ts'
const id='a'.repeat(32),other='b'.repeat(32),hash='c'.repeat(64)
const running={operation_id:id,state:'running'}
test('backup identity reconciliation accepts only exact boolean status and true initialization',()=>{
  assert.equal(backupIdentityStatus({initialized:false}),false);assert.equal(backupIdentityStatus({initialized:true}),true);backupIdentityInitialized({initialized:true})
  for(const raw of [{},{initialized:1},{initialized:'true'},{initialized:true,key_id:'forbidden'}])assert.throws(()=>backupIdentityStatus(raw))
  assert.throws(()=>backupIdentityInitialized({initialized:false}))
})
test('only definite non-conflict 4xx permits a fresh explicitly selected attempt',()=>{
  assert.equal(storageWriteUncertain({status:422,code:'invalid_selection'}),false)
  assert.equal(storageWriteUncertain({status:400,code:'invalid_input'}),false)
  assert.equal(storageWriteUncertain({status:409,code:'backup_identity_missing'}),false)
  assert.equal(storageWriteUncertain({status:409,code:'storage_busy'}),false)
  for(const e of [null,{status:409,code:'unknown_conflict'},{status:500,code:'internal_error'},{status:null,code:'request_failed'},{status:400,code:'selection_changed'}])assert.equal(storageWriteUncertain(e),true)
})
test('capabilities reject paths, numbers, traversal, separators and invalid hex',()=>{
  assert.equal(capabilityId(id),id)
  for(const v of ['/private/tmp/a','../a',12,'A'.repeat(32),'a'.repeat(31),`${id}?x`])assert.throws(()=>capabilityId(v))
})
test('recipient preview requires exact public identity and fingerprint without metadata omission',()=>{
  assert.deepEqual(storagePreview({request_id:id,fingerprint:hash}),{requestId:id,fingerprint:hash})
  for(const value of [{request_id:id},{request_id:id,fingerprint:'123'},{request_id:id,fingerprint:hash,path:'/a'}])assert.throws(()=>storagePreview(value))
})
test('export command carries explicit confirmed request identity and typed capabilities only',()=>{
  const selections={book:id,request:other,'export-destination':'d'.repeat(32)}
  assert.throws(()=>storageCommand('export',selections))
  assert.deepEqual(storageCommand('export',selections,{requestId:id,fingerprint:hash}),{path:'/api/storage/exports',payload:{book:id,request:other,destination:'d'.repeat(32),confirmed_request_id:id,confirmed_fingerprint:hash}})
  assert.throws(()=>storageCommand('export',{...selections,book:'/a'},{requestId:id,fingerprint:hash}))
})
test('restore/import commands cannot overwrite or select a book and recovery kinds are explicit',()=>{
  assert.deepEqual(storageCommand('restore',{backup:id,'book-destination':other}),{path:'/api/storage/restores',payload:{backup:id,destination:other}})
  assert.deepEqual(storageCommand('import',{package:id,request:other,'book-destination':id}).payload,{package:id,request:other,destination:id})
  assert.deepEqual(storageCommand('recover-imports',{'recovery-parent':id}).payload,{kind:'imports',parent:id})
  assert.deepEqual(storageCommand('recover-exports',{'recovery-parent':id,book:other}).payload,{kind:'exports',book:other,parent:id})
  assert.deepEqual(requiredSelections['container-import'],['request','book-destination'])
  assert.throws(()=>storageCommand('container-import',{request:id,'book-destination':other}))
})
test('operation status matches job identity and exclusive lifecycle fields',()=>{
  assert.equal(storageStatus(running,'export',id).state,'running')
  assert.throws(()=>storageStatus(running,'export',other))
  assert.throws(()=>storageStatus({...running,result:null},'export'))
  assert.throws(()=>storageStatus({...running,state:'failed',error_code:'raw /private/secret'},'export'))
  assert.equal(storageStatus({...running,state:'failed',error_code:'storage_operation_failed'},'export').state,'failed')
  assert.throws(()=>storageStatus({...running,state:'complete'},'export'))
})
test('export result validates exact counters above safe Number and selection semantics',()=>{
  const result={package_id:id,attachment_count:'9007199254740993',attachment_bytes:'18446744073709551615',cleanup_warning:false,selection_required:false}
  const next=storageStatus({operation_id:id,state:'succeeded',result},'export',id)
  assert.ok(next.result.includes('附件數：9,007,199,254,740,993'))
  assert.throws(()=>storageResult({...result,attachment_count:9007199254740992},'export'))
  assert.throws(()=>storageResult({...result,attachment_bytes:'18446744073709551616'},'export'))
  assert.throws(()=>storageResult({...result,selection_required:true},'export'))
  assert.throws(()=>storageResult({...result,attachment_count:undefined},'export'))
})
test('import and restore success must require explicit later book selection',()=>{
  const result={package_id:id,attachment_count:'0',cleanup_warning:true,selection_required:true}
  assert.ok(storageResult(result,'container-import').some(text=>text.includes('未自動開啟')))
  assert.ok(storageResult(result,'import').some(text=>text.includes('清理警告')))
  assert.throws(()=>storageResult({...result,selection_required:false},'import'))
  assert.throws(()=>storageResult({...result,attachment_bytes:'0'},'import'))
  assert.equal(storageResult({attachment_count:'0',attachment_bytes:'0',cleanup_warning:false,selection_required:true},'restore').length,3)
})
test('recovery results distinguish verified active and preserved unknown work',()=>{
  const result={removed_count:'0',verified_count:'2',active_count:'1',unrecognized_count:'4',cleanup_warning:false,selection_required:false}
  assert.ok(storageResult(result,'recover-imports').some(text=>text.includes('未辨識工作數：4')))
  assert.throws(()=>storageResult({...result,active_count:'-1'},'recover-exports'))
  assert.throws(()=>storageResult({...result,verified_count:'01'},'recover-imports'))
  assert.throws(()=>storageResult({...result,unknown:true},'recover-imports'))
})
test('verified backup remains successful when retention warning exists',()=>{
  const result={backup_name:'backup-20261023',verified:true,attachment_count:'0',attachment_bytes:'0',retention_warning:null,preserved_unrecognized:'2'}
  assert.ok(backupResult(result).some(text=>text.includes('已獨立驗證')))
  assert.ok(backupResult({...result,retention_warning:'opaque warning',preserved_unrecognized:null}).some(text=>text.includes('備份已成功')))
  assert.throws(()=>backupResult({...result,retention_warning:'warning'}))
  assert.throws(()=>backupResult({...result,verified:false}))
  assert.throws(()=>backupResult({...result,backup_name:'../../file'}))
  assert.throws(()=>backupResult({...result,attachment_bytes:0}))
})
