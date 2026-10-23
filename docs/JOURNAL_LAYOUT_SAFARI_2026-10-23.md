# 分錄新版面 Safari 驗收（2026-10-23）

依使用者提供的傳統普通日記簿圖片調整分錄編輯及單筆已過帳檢視：日期、科目／摘要、借方 TWD、貸方 TWD；沒有 PR 欄。操作欄僅供編輯列使用。既有選填參考編號保留於表格上方，並非 PR 欄。

## 實際 Safari 操作

使用 `local_acceptance frontend/dist --chooser-ui`，一次性假資料、記憶體假金鑰，不接觸真實帳簿或 Keychain。最終 fixture token 為 `f36fd602304fa18351046067ddadc6bd`。

- 明確選取並開啟 book，進入分錄編輯。
- 第一列填驗收費損、借方 1500、列摘要「購置用品」。第二列填驗收負債、貸方 1500、列摘要「應付供應商」。整筆說明「新版分錄版面驗收（假資料）」。
- 畫面確認日期在首列；科目與列摘要同欄，貸方科目及其摘要縮排；整筆說明位於科目列下方。金額靠右、借貸分欄，底部摘要顯示借貸各 1,500、差額 0。沒有 PR 欄。
- 明確儲存後為草稿 2、版本 1，輸入值保留。再點過帳並獨立確認，成功成為已過帳分錄 2。
- 已過帳科目、金額、列摘要及整筆說明以文字顯示，沒有表內可編輯控制項；借貸各 1,500，貸方縮排與日期維持，整筆說明以括號顯示於下方。
- 測試視窗關閉、一次性服務停止；保留使用者原本 Safari 起始頁面。

驗收時發現前版日期僅在表外，已補上表內日期欄；原先已過帳金額是停用輸入框，已改為純文字後重新實際驗收。這是 Mac Safari 單筆編輯及檢視驗收，不宣稱普通日記簿報表的多筆群組或 iPad 已完成同輪視覺驗收。

外置畫面與驗證紀錄位於 `deliverables/journal-layout-verification/`。原有 candidate5 為先前版封裝，不含本次前端變更，未重新封裝。正式簽章持續暫緩。

## 完整驗證

`./scripts/verify.sh` 通過：684 個 Rust、141 個工具、186 個前端測試，9 個 Rust 測試刻意忽略，0 失敗；型別檢查與前端 production build 通過。`cargo build --release --locked` 通過，兩份 lockfile 均未修改。新增回歸檢查確認表頭沒有 PR、日期與說明位於表內、貸方縮排、已過帳表內沒有輸入框及金額以文字顯示。

## 新開發封裝 candidate6

後續已將本次介面製成 `kAccounting-Extra-01-development-candidate6`（macos-arm64），來源為 `c760c5a` 完整提交。前述 candidate5 未重包的敘述為 Safari 驗收當時狀態；目前 candidate5 保留，candidate6 包含新版介面。

- manifest SHA-256：`209a0c3daa4f9914eda3ead82b195a65ae0c022a30012b6c3028b5e21c0c1f06`。
- 封裝含 6 個 runtime 檔案；外置 digest 驗證、權限／架構／完整性檢查成功，採用正確 `serve-native-chooser` 啟動資訊。
- 16 個封裝測試通過；重新執行 locked Release 與 frontend build 成功。
- 實際執行封裝內 backend，以全新空白 private collection 啟動；HTTP 送出的 index 與所有 CSS／JS 與封裝逐 byte 相同。集合保持空白，必須明確選簿，helper 僅 pin 未開面板，SIGINT 正常關閉 exit 0。
- 未進行真實 Keychain、帳簿、登入／重開機或正式部署驗收；沒有 Developer ID 簽章或公證。

封裝及獨立證據保留於工作區外 `deliverables/kAccounting-Extra-01-development-candidate6` 與 `deliverables/candidate6-verification/DELIVERY.json`。
