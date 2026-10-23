# 短項目補驗 — 2026-10-23 UI5

本輪三個智慧體分別盤點剩餘工作、同步交接文件及執行容量驗收；主智慧體操作
Safari。僅使用拋棄式帳簿與記憶體假金鑰，沒有真實財務資料或系統設定變更。

## 實際新增證據

- Safari 原生復原上層資料夾面板按 Cancel 後，顯示「已取消選擇；沒有開始財務作業。」；
  尚未選擇資料夾，作業確認仍停用。其他目的的原生面板取消不因此全部視為已驗。
- 同一測試服務的兩個 Safari 分頁：B 明確從 `book` 切換到 `empty`，A 保留舊畫面。
  A 再按原生復原資料夾選擇時，沒有開啟面板，介面同步到 `empty` 並清除作業輸入。
  這是 UI 觀察；沒有擷取 HTTP 狀態，不冒稱已補驗舊頁 upload/download。
- Release 最大附件 250 MiB 加密與驗證：1 passed，測試耗時 2.05 秒。
- Release 最大收件者移轉套件 250 MiB 匯出／匯入：1 passed，測試耗時 13.13 秒。
  核對來源資料庫與附件密文保留、目的端使用不同金鑰、串流內容雜湊及資料庫完整性。
  這兩項是 routine ignored 之外明確執行的額外測試，不與完整套件總數累加。

容量測試精確命令、原始 log、時間與 SHA-256 保存於持久交付根目錄的
`deliverables/ui5-verification/release-capacity-receipt.json`。
測試程式來源仍為 `35d6d0f35aea2962f1322967959d559cf09d1294`。

## 文件及交付

更新 START_ON_NEW_MAC、HANDOFF_NEXT_MAC、DEVELOPMENT_PLAN 的目前狀態，
保留並標示舊 checkpoint，修正唯讀 UI／四碼代碼等已被取代的要求。
驗收清單只補本輪實際觀察，不把部分情境直接勾成全部完成。

本輪無程式或 lockfile 修改，`git diff --check` 通過，無需重新建置同一份 binary。
完整驗證基準仍是 UI4 的 669 Rust／141 工具／141 前端、9 例行忽略及 locked Release。
candidate3 開發封裝及其 `ui4-verification/DELIVERY.json` 保持有效。
本輪測試服務已停止，3000 埠無 listener，僅關閉本輪建立的測試視窗。

## 接續順序

1. 儲存草稿附件的上傳／取消／移除，以及未儲存編輯禁寫的 Safari 驗收。
2. 資料夾格式移轉包原生匯入與更多面板取消情境。
3. Safari 大附件、舊頁 upload/download 的端到端驗收。
4. 需要使用者操作的私人代理正式部署、真實 Keychain、登入／重開機及實體跨 Mac。

正式簽章仍依使用者指示暫緩；沒有安裝永久服務或操作真實金鑰。
