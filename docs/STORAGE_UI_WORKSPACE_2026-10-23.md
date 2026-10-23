# 備份、還原與跨 Mac 移轉 UI — 2026-10-23

## 本輪補充：舊格式還原帳簿的備份身分

依 ADR 0031，只有目前帳簿明確備份回報 `backup_identity_missing`，介面才提供
獨立「設定此帳簿的備份身分」動作。使用者另勾 checkbox 並確認後才送
`POST /api/backups/identity {}`；嚴格核對 `201 {initialized:true}`。
不自動初始化、不更換裝置金鑰、不繼承舊備份的清理權限，也不自動建立備份。
設定成功後，同一已選帳簿可立即另行明確確認備份，不必重新開簿。

初始化結果未知或回報 `backup_identity_exists` 時，禁止再次 POST，提供明確
`GET /api/backups/identity` 核對精確 `{initialized:boolean}`。只有 true 才解鎖備份，
false 則仍須重新 checkbox 明確設定。損毀身分、格式不符及核對失敗保持鎖定，
不得把未知狀態當成缺檔。GET 不自動重送初始化或備份。

中斷復原介面另修正：工作狀態回報 `storage_operation_not_found` 時，保留原 ID，
標示可能曾重啟與結果未知，允許明確轉入已認證復原流程；不永遠困在 running，
也不重新提交原匯入／匯出。復原本身失敗／未知結果不偽造成功計數、不自動重試；
成功結果分別保留已驗證、進行中及未辨識工作數。

此增量實跑 **25 項 targeted 測試（11 模型、14 模擬 DOM）**、TypeScript 與
diff 檢查通過；下文 17 項是前輪歷史 checkpoint。Safari 舊格式還原後備份及
中斷復原實際驗收由協調智慧體另行記錄，不以 DOM 測試冒稱實機完成。

此輪新增 `storage-model.ts`、`storage-ui.ts` 及模型／模擬 DOM 測試。
維持帳簿 generation 固定、明確確認、精確整數與不覆寫目的地的既有契約。
此文件記錄介面能力與測試範圍，不代表永久服務或真實金鑰驗收完成。

## 操作範圍

| 操作 | 既有後端接點 | 介面限制 |
| --- | --- | --- |
| 手動加密備份 | `POST /api/backups` | 明確確認，獨立驗證；保留清理警告的成功結果，不偷偷補備份身分 |
| 建立收件請求 | `POST /api/storage/recipient-requests` | 原生選擇全新目的地；顯示公開 request ID 與指紋 |
| 匯出 | `POST /api/storage/exports` | 原生選擇來源帳簿、請求及新目的地；預覽後明確核對完整請求識別與公鑰指紋 |
| 資料夾移轉包匯入 | `POST /api/storage/imports` | 原生選擇包、請求及新帳簿目的地 |
| `.kagextra` 單檔匯入 | `POST /api/storage/container-imports` | 原生請求／目的地 capabilities；File body 由共用傳輸介面串流，不 JSON/base64 緩衝 |
| 加密備份還原 | `POST /api/storage/restores` | 原生備份／新帳簿目的地；不取代目前帳簿 |
| 中斷匯入／匯出復原 | `POST /api/storage/recoveries` | 明確選擇工作上層；匯出另選來源帳簿，分列清理、已驗證、進行中與未辨識數 |

工作狀態用既有 `GET /api/storage/operations/{id}` 查詢。
後端僅提供 running／succeeded／failed，畫面使用不定進度，不虛構百分比。
只有成功且已驗證的匯出工作可交給 Safari 下載；按鈕不宣稱檔案已完成寫入。
使用者先到 Downloads 核對，再手動移動至外部媒體。

## 原生與傳輸邊界

`StorageContext` 由主程式提供以下受信任邊界：

- `selectStorage(kind, signal)`：有限種類、真實原生選擇；只回 opaque capability 或取消。
- `downloadStorage(operationId, signal)`：共用 generation guard 與受驗證下載。
- `importStorageContainer(file, requestCapability, destinationCapability, signal)`：加密 File body。

UI 不接收使用者輸入的路徑、Keychain ID、金鑰、密碼或 overwrite 旗標；不以
browser file path 偽裝原生選擇。缺少原生／binary hook 時明示不可用，不能回退至
不受信任的路徑或普通 JSON 檔案上傳。原生 localhost bridge 的註冊／Origin／
generation／私人入口限制由本輪主程式與後端整合及其 ADR 另行記錄。

收件請求／capability 不寫入 localStorage 或其他瀏覽器持久儲存。
不同帳簿／generation 的回應不採用，離開時停止 UI polling；後端已受理工作
仍自行持有生命週期，不宣稱關閉網頁即可取消或回滾。

## 未知結果與明確復原

網路中斷、格式不符、伺服器錯誤與未核對衝突不自動重送。
已有工作 ID 時可重新讀取同一工作；沒有 ID 時顯示保留來源／目的地的核對說明，
使用者明確記下狀態後可允許離開，但原畫面仍不允許重送。
匯入／匯出可另經明確確認轉到中斷復原，要求重新選擇工作上層，不重送原作業。
已明確拒絕的非衝突 4xx，以及已知未受理的備份身分缺少／忙碌／容量錯誤，
清空 capabilities，允許重新選擇後再次明確提交；沒有計時重試。

匯入／還原成功必須回報 `selection_required=true`，且介面明列「另行選簿」。
不自動切換、開啟或記住新帳簿。結果欄位遺漏、job ID 不符、錯誤生命週期欄位、
未知欄位、非精確十進位計數與錯誤 selection 語意皆拒絕顯示成功。

## 驗證與尚未完成

本工作單元實跑 TypeScript 檢查與 17 項 targeted 測試通過（10 模型、7 模擬 DOM）。
涵蓋精確大整數、capability／指紋格式、確認 payload、未知回應／未知欄位、
不覆寫／不選簿、成功但有 retention warning、原生取消、明確 checkbox、
未知提交不重試、明確轉至中斷復原而不重送、已知備份身分拒絕、匯出 job 成功後
才允許下載、不同 job ID 回應不會被當成成功。
完整整合驗證與 Safari／原生互動以協調智慧體實際結果為準。
同一 Mac、假測試金鑰的 fixture `06e9107a4a65c6b0b6da2d55201a2b4d` 已實測
原生建立 `recipient-request.json`、核對 request ID／完整指紋後明確匯出、Safari
Downloads 完成約 112 KB `.kagextra`、原生選擇同一 request 與全新
`Safari-Imported-Book` 目的地、browser File 單檔匯入成功。驗證的 package ID
為 `cfbec3d9808e476413b186f4f05f1232`，零附件；匯入後仍選擇來源帳簿。
另更新候選清單並明確選擇匯入帳簿後，試算表借貸各精確顯示
`9,007,199,254,740,993`。來源 canonical temp root 為
`/private/var/folders/.../T/ka-http...`，不能錯寫成 `/private/tmp`。
詳細公開 request／fingerprint／job 證據見 [本輪總結](BUILD_STORAGE_UI_2026-10-23.md)。

全新電腦初始化由另一工作單元整合受信任原生 `POST /api/native/books`：
空物件輸入，回傳 `{book_id: string|null, selection_required: true}`，取消回傳 null，
不得自動變更目前 selection。原生 first-book 後端及主程式「建立帳簿」按鈕已實作，
流程完成後重新核對候選清單，由使用者另行選簿。可信初始集合位置修正後，
Safari 已實測 `Safari-New-Book` 成功建立，UI 仍顯示尚未選簿，沒有自動開啟。
同 fixture 後續亦實測備份還原：原生選擇管理備份內的 `backup.kabackup` 與全新
`Safari-Restored-Book`，明確確認後工作 `698027988cdcc806533836cdee4734de` 成功，
零附件／零位元組、`selection_required=true`，來源帳簿保持選擇。沒有另外開啟
還原帳簿或核對其報表。UI 特別說明不可選外層 managed 資料夾。
實體跨 Mac、大附件移轉、資料夾包匯入及中斷復原仍須各自實機驗收，
不能以上述同 Mac 零附件單檔鏈或空白初始化實測替代。
本工作未改變簽章暫緩、真實 Keychain、永久私人部署與登入／重開機待驗收狀態。

最新候選驗證紀錄：664 Rust／9 刻意忽略、141 Python、132 前端與 locked Release。
工具數包含三項 native startup 封裝回歸；較早 138 工具為歷史 checkpoint。
帶 helper 的新包採 `serve-native-chooser` 與 exact helper argument，舊包 verify
明示 `legacy_native_startup`。最新完成狀態、hash 與 commit 綁定以外置
`deliverables/ui3-verification/DELIVERY.json` 為準。
最大附件 HTTP 來回／未宣告超額清理另以 Release 明確執行一項 ignored 測試通過，
不等同 Safari 大檔案或跨 Mac 大附件實測。
