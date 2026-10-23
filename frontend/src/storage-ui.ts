import { ApiError, apiGuidance } from './session-client.ts'
import { backupIdentityInitialized, backupIdentityStatus, backupResult, capabilityId, requiredSelections, storageCommand, storagePreview, storageResult, storageSelectionPrompt, storageStatus, storageWriteUncertain, type StorageAction, type StoragePreview, type StorageSelectionKind, type StorageStatus } from './storage-model.ts'
export interface StorageContext {
  getSelection():{bookId:string;selectionId:string}|null
  request(path:string,signal:AbortSignal,method?:string,payload?:unknown):Promise<unknown>
  onSelectionChanged():void
  selectStorage?(kind:StorageSelectionKind,signal:AbortSignal):Promise<{capabilityId:string}|null>
  downloadStorage?(operationId:string,signal:AbortSignal):Promise<void>
  importStorageContainer?(file:File,requestCapability:string,destinationCapability:string,signal:AbortSignal):Promise<unknown>
}
const labels:Record<StorageAction,string>={backup:'建立加密備份','recipient-request':'本機建立收件請求',export:'匯出給另一台 Mac',import:'匯入資料夾移轉包','container-import':'匯入 .kagextra 檔案',restore:'從加密備份還原','recover-imports':'復原中斷匯入','recover-exports':'復原中斷匯出'}
const selectionLabels:Record<StorageSelectionKind,string>={book:'來源帳簿',request:'收件請求檔',package:'移轉包資料夾',backup:'加密備份包（backup.kabackup）','request-destination':'新的收件請求檔','export-destination':'新的匯出資料夾','book-destination':'新的帳簿目的地','recovery-parent':'中斷工作所在上層資料夾'}
function el<K extends keyof HTMLElementTagNameMap>(tag:K,text='',className=''):HTMLElementTagNameMap[K] {const n=document.createElement(tag);n.textContent=text;n.className=className;return n}
export function mountStorageUI(host:HTMLElement,context:StorageContext):{dispose():void;hasUnsavedChanges():boolean} {
  const root=el('section','','storage-ui'),status=el('p','','status-line'),form=el('div','','storage-controls'),result=el('div','','storage-result'),confirm=el('div','','storage-confirmation'),select=el('select'),controller=new AbortController()
  status.setAttribute('role','status');status.setAttribute('aria-live','polite')
  const selection=context.getSelection();let disposed=false,busy=false,uncertain=false,acknowledged=false,picking=false,polling=false,downloadBusy=false,operation:StorageStatus|null=null,action:StorageAction='backup',preview:StoragePreview|undefined,file:File|null=null,timer:ReturnType<typeof setTimeout>|null=null
  let selections:Partial<Record<StorageSelectionKind,string>>={}
  let identity:'unseen'|'missing'|'initialized'|'uncertain'='unseen',operationLost=false
  for(const [key,label] of Object.entries(labels)) {const option=el('option',label);option.value=key;select.append(option)}
  select.setAttribute('aria-label','儲存作業種類')
  root.append(el('h2','備份、還原與跨 Mac 移轉'),el('p','備份保留於目前帳簿管理的加密備份區。還原與匯入只建立新目的地，不覆寫、不自動切簿；不要刪除來源、收件請求或中斷工作。'),el('p','跨 Mac：目的端建立收件請求 → 來源端核對請求識別與公鑰指紋後匯出 → Safari 先下載至 Downloads → 手動移動 → 目的端匯入。私人金鑰由本機系統管理，不輸入密碼。'),select,form,confirm,status,result);host.append(root)
  const current=():boolean=>{const now=context.getSelection();return !disposed&&!!selection&&now?.bookId===selection.bookId&&now.selectionId===selection.selectionId}
  function error(value:unknown):void {
    if(!current())return
    if(value instanceof ApiError&&value.code==='selection_changed'){context.onSelectionChanged();return}
    status.textContent=value instanceof ApiError?apiGuidance(value):'無法核對作業結果；請保留所有來源與中斷資料，不要重送。'
  }
  function button(label:string,callback:()=>void,disabled=false):HTMLButtonElement {const n=el('button',label,'secondary-button');n.type='button';n.disabled=disabled;n.addEventListener('click',callback);return n}
  function output(lines:string[]):void {result.replaceChildren(...lines.map(line=>el('p',line)))}
  function clearConfirmation():void {confirm.replaceChildren()}
  function reset():void {selections={};preview=undefined;file=null;operation=null;operationLost=false;uncertain=false;acknowledged=false;result.replaceChildren();clearConfirmation()}
  function render():void {
    form.replaceChildren();select.disabled=busy||picking||uncertain||operation?.state==='running'
    if(action==='backup')form.append(el('p','建立獨立驗證的加密檢查點，保留最新十份已驗證管理備份；不會替舊帳簿偷偷建立備份身分。'))
    if(action==='backup'&&identity==='missing')form.append(el('p','此舊格式還原帳簿尚未設定本機備份。確認後只管理此帳簿新建立的備份；舊備份與原有加密金鑰都會保留。設定完成後仍須另行確認建立備份。'),button('設定此帳簿的備份身分',prepareIdentity,busy||uncertain||picking))
    if(action==='backup'&&identity==='uncertain')form.append(el('p','備份身分設定結果尚未核對，不得重送初始化。'),button('核對備份身分狀態',()=>void reconcileIdentity(),busy||picking))
    if(action==='restore')form.append(el('p','選擇管理備份資料夾內的 backup.kabackup，不選外層 managed 資料夾。'))
    if(action!=='backup'&&!context.selectStorage)form.append(el('p','目前執行環境未提供受信任的原生選擇器；此流程不可用。請使用具原生工具的本機版本，不能在網頁輸入路徑。'))
    for(const kind of requiredSelections[action]) {
      const done=selections[kind]!==undefined
      form.append(button(`${selectionLabels[kind]}：${done?'已選擇（重新選擇）':'選擇…'}`,()=>void pick(kind),!context.selectStorage||busy||picking||uncertain||operation?.state==='running'))
    }
    if(action==='container-import') {
      const wrapper=el('label','選擇已下載的 .kagextra 檔案'),input=el('input');input.type='file';input.accept='.kagextra';input.disabled=busy||picking||uncertain||!context.importStorageContainer
      input.addEventListener('change',()=>{file=input.files?.item(0)??null;clearConfirmation();result.replaceChildren();render()});wrapper.append(input);form.append(wrapper)
      if(file)form.append(el('p',`已選擇檔案：${file.name}`))
      if(!context.importStorageContainer)form.append(el('p','此環境沒有加密單檔串流匯入介面；不會以 JSON 或檔案路徑代替。'))
    }
    if(action==='export'&&preview) form.append(el('p',`請求識別：${preview.requestId}`),el('p',`收件公鑰指紋：${preview.fingerprint}`),el('p','請與目的端顯示的識別及完整指紋逐字核對。'))
    const ready=requiredSelections[action].every(kind=>!!selections[kind])&&(action!=='export'||!!preview)&&(action!=='container-import'||!!file&&!!context.importStorageContainer)
    form.append(button('檢查並確認此作業',prepare,!ready||busy||picking||uncertain||operation!==null||(action==='backup'&&identity==='missing')))
    if(operation)form.append(el('p',`工作識別：${operation.operationId}`))
    if(operation?.state==='running'&&!operationLost) {const progress=el('progress');progress.setAttribute('aria-label','後端作業進行中（未提供百分比）');form.append(progress,el('p','後端工作會繼續執行；關閉網頁不代表取消。'),button('重新查詢進度',()=>void poll(),polling))}
    if(operationLost)form.append(el('p','此服務已無原工作識別，可能曾重啟；結果未知。請保留工作識別與來源，改用已認證中斷復原，不重送原操作。'))
    if(operation?.state==='succeeded'&&action==='export')form.append(button('交給 Safari 下載至 Downloads',()=>void download(),!context.downloadStorage||downloadBusy))
    if(uncertain&&operation?.state!=='running')form.append(el('p','結果尚未確認。不得重送；若已有工作識別，請重新查詢；服務重啟後使用中斷復原。'))
    if(uncertain&&(!operation||operationLost)) {
      form.append(button('已記下待核對狀態，允許離開此畫面',()=>{if(window.confirm('請先記下此次作業種類與已選來源／目的地。工作可能已受理，不能盲目重送。離開後先人工核對；服務重啟後才使用適合的中斷復原。確定允許離開？')){acknowledged=true;status.textContent='待核對作業仍保留且未重送。現在可離開畫面；請先核對原目的地與來源，再考慮後續操作。';render()}}))
      if(action==='import'||action==='container-import'||action==='export')form.append(button('確認轉至中斷復原（不重送原作業）',()=>{if(window.confirm('先確認原作業不再進行，或服務已重啟。中斷復原會檢查已認證工作；不會重送原匯入／匯出。繼續選擇復原上層？')){const next=action==='export'?'recover-exports':'recover-imports';reset();action=next;select.value=next;status.textContent='請重新選擇中斷工作上層；沒有重送原作業。';render()}}))
    }
    if(operation&&operation.state!=='running'&&!uncertain)form.append(button('開始另一項作業',()=>{reset();render();status.textContent=storageSelectionPrompt(action,true)}))
  }
  function prepareIdentity():void {
    if(!current()||busy||uncertain||identity!=='missing')return
    clearConfirmation();const checkbox=el('input');checkbox.type='checkbox';const label=el('label','我確認只為目前帳簿設定本機備份，保留舊備份與原有加密金鑰');label.prepend(checkbox)
    const commit=button('確認建立備份身分',()=>{if(checkbox.checked)void initializeIdentity()},true);checkbox.addEventListener('change',()=>{commit.disabled=!checkbox.checked})
    confirm.append(el('p','這是獨立設定，不是備份；不會自動執行後續備份。'),label,commit,button('取消',clearConfirmation))
  }
  async function initializeIdentity():Promise<void> {
    if(!current()||busy||uncertain||identity!=='missing')return
    busy=true;clearConfirmation();render()
    try{const raw=await context.request('/api/backups/identity',controller.signal,'POST',{});if(!current())return;backupIdentityInitialized(raw);identity='initialized';status.textContent='備份身分已建立；目前帳簿保持選擇。請另按檢查並確認此作業，明確建立備份。'}
    catch(value){if(current()){if(value instanceof ApiError&&!storageWriteUncertain(value)&&value.code!=='backup_identity_exists'){identity='missing'}else{identity='uncertain';uncertain=true}error(value)}}
    finally{busy=false;if(current())render()}
  }
  async function reconcileIdentity():Promise<void> {
    if(!current()||busy||identity!=='uncertain')return
    busy=true;clearConfirmation();render()
    try{const raw=await context.request('/api/backups/identity',controller.signal);if(!current())return;const exists=backupIdentityStatus(raw);identity=exists?'initialized':'missing';uncertain=false;acknowledged=false;status.textContent=exists?'已核對備份身分存在；未重送初始化。請另行明確備份。':'已核對仍缺少備份身分；未重送初始化。可重新明確確認設定。'}catch(value){error(value)}finally{busy=false;if(current())render()}
  }
  async function pick(kind:StorageSelectionKind):Promise<void> {
    if(!context.selectStorage||!current()||busy||picking||uncertain||operation?.state==='running')return
    picking=true;clearConfirmation();render();status.textContent='請在 macOS 原生面板選擇，或取消。'
    try {const value=await context.selectStorage(kind,controller.signal);if(!current())return
      if(value===null){status.textContent='已取消選擇；沒有開始財務作業。';return}
      const cap=capabilityId(value.capabilityId);delete selections[kind];if(kind==='request')preview=undefined
      if(kind==='request'&&action==='export') {const next=storagePreview(await context.request(`/api/storage/selections/${cap}/request`,controller.signal));if(!current())return;preview=next}
      selections[kind]=cap;status.textContent='已完成原生選擇；尚未開始作業。'
    }catch(value){error(value)}finally{picking=false;if(current())render()}
  }
  function prepare():void {
    if(!current()||busy||picking||uncertain||operation)return
    clearConfirmation()
    const agreement=el('input');agreement.type='checkbox';const label=el('label',action==='export'?'我已與目的端核對完整請求識別與公鑰指紋':'我確認只執行上述作業，保留來源且不覆寫現有目的地');label.prepend(agreement)
    const commit=button(`確認${labels[action]}`,()=>{if(agreement.checked)void submit()},true);agreement.addEventListener('change',()=>{commit.disabled=!agreement.checked})
    confirm.append(el('p',`${labels[action]}：此作業可能讀取本機金鑰；開始後不因網頁斷線取消。`),label,commit,button('取消',clearConfirmation))
  }
  async function submit():Promise<void> {
    if(!current()||busy||uncertain||operation)return
    busy=true;clearConfirmation();render();status.textContent='正在提交一次作業，請勿重送…';let sent=false
    try {
      if(action==='container-import') {
        if(!context.importStorageContainer||!file)throw new Error('unavailable')
        const request=capabilityId(selections.request),destination=capabilityId(selections['book-destination']);sent=true
        const raw=await context.importStorageContainer(file,request,destination,controller.signal);if(!current())return
        const lines=storageResult(raw,action);output(lines);status.textContent='單檔匯入已完成並驗證；請另行選簿。';selections={};file=null;operation=null
      } else {
        const command=storageCommand(action,selections,preview);sent=true;const raw=await context.request(command.path,controller.signal,'POST',command.payload);if(!current())return
        if(action==='backup') {output(backupResult(raw));identity='initialized';status.textContent='加密備份已完成並驗證。'}
        else {const next=storageStatus(raw,action);if(next.state!=='running')throw new Error('invalid admission');operation=next;selections={};preview=undefined;status.textContent='作業已受理，正在查詢進度…';schedule()}
      }
    }catch(value){if(current()&&action==='backup'&&value instanceof ApiError&&value.code==='backup_identity_missing')identity='missing';if(sent){uncertain=storageWriteUncertain(value instanceof ApiError?value:null);if(!uncertain){selections={};preview=undefined;file=null}}error(value)}finally{busy=false;if(current())render()}
  }
  function schedule():void {if(timer!==null)clearTimeout(timer);if(current()&&operation?.state==='running')timer=setTimeout(()=>{timer=null;void poll()},1000)}
  async function poll():Promise<void> {
    if(!current()||polling||!operation||operation.state!=='running')return
    polling=true;const id=operation.operationId
    try {const next=storageStatus(await context.request(`/api/storage/operations/${id}`,controller.signal),action,id);if(!current())return;operation=next;uncertain=false
      if(next.state==='running'){status.textContent='後端作業進行中；未提供百分比。';schedule()}
      else {output(next.result);status.textContent=next.state==='succeeded'?'作業已完成。':'作業失敗；請保留來源並檢查中斷工作。'}
    }catch(value){uncertain=true;if(value instanceof ApiError&&value.status===404&&value.code==='storage_operation_not_found'){operationLost=true;if(timer!==null){clearTimeout(timer);timer=null}}error(value)}finally{polling=false;if(current())render()}
  }
  async function download():Promise<void> {if(!current()||!context.downloadStorage||downloadBusy||operation?.state!=='succeeded'||action!=='export')return;downloadBusy=true;render();try{await context.downloadStorage(operation.operationId,controller.signal);if(current())status.textContent='已交給 Safari 下載；請到 Downloads 確認完成後，手動移動至外部媒體。'}catch(value){error(value)}finally{downloadBusy=false;if(current())render()}}
  select.addEventListener('change',()=>{if(busy||picking||uncertain||operation?.state==='running')return;action=select.value as StorageAction;reset();status.textContent=storageSelectionPrompt(action);render()})
  status.textContent='請選擇作業；不會自動備份、移轉或開啟其他帳簿。';render()
  return {dispose(){disposed=true;controller.abort();if(timer!==null)clearTimeout(timer);root.remove()},hasUnsavedChanges(){return busy||picking||(!acknowledged&&(uncertain||Object.keys(selections).length>0||file!==null))||downloadBusy||(operation?.state==='running'&&!operationLost)}}
}
