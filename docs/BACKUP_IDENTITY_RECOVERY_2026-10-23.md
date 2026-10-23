# 舊格式還原帳簿備份身分與中斷復原 — 2026-10-23（台北）

本輪依使用者指示，先補足舊格式還原帳簿的明確備份身分設定與中斷復原驗收。
不修改會計政策、裝置金鑰、現有備份清理權限；不操作真實財務帳簿或既有金鑰。
此文件將自動化與 Safari 實機證據分開，未完成的驗收不提前填通過。

## 本輪變更

- Session API 新增明確 `POST /api/backups/identity {}` 與核對狀態的
  `GET /api/backups/identity`，不在開簿、還原或備份失敗時偷偷初始化。
  使用者必須先明確選簿；成功後同一 session 即可另行備份，不必關閉重開。
- 身分為新的管理備份 namespace，不繼承其他 namespace 的 pruning 權限。
  初始化使用已持有的本機裝置金鑰，不讀取／更換其他金鑰，不覆寫既有身分。
- UI 只有實際收到 `backup_identity_missing` 才顯示獨立設定入口，另需 checkbox
  與明確確認。設定成功不自動備份，使用者仍須另行確認建立備份。
- 初始化未知結果或存在衝突時禁止重送，提供明確 GET 狀態核對。
  只有有效 true／false 回應才能解除相應鎖定；損毀／錯誤資料不當成缺檔。
- 工作 ID 因服務重啟而失效時，保留原 ID 與未知狀態，提供明確轉入中斷復原；
  不無限困在 running、不重送原匯入／匯出。復原失敗／未知結果不自動重試。

API 合約見 [API](API.md)，身分與保留規則見 [ADR 0031](decisions/0031-managed-backup-lifecycle.md)。

## 已完成的單元驗證

儲存 UI 單元實跑 **25 項 targeted 測試（11 模型、14 模擬 DOM）**，另
TypeScript 與 diff 檢查通過。涵蓋：missing 才顯示入口、獨立 checkbox、
設定成功不自動備份、同選簿下一次明確備份、未知 POST 不重送、GET true／false
核對、malformed status 保持鎖定、exists 衝突核對、工作 ID 遺失後復原、
復原失敗／未知結果與未辨識內容計數。

本輪完整 `./scripts/verify.sh` 已通過：**669 Rust、141 Python、141 前端測試**，
**9 項刻意忽略、0 失敗**，TypeScript／production build 成功；
`cargo build --release --locked` 成功。最後提示文案另做前端 check，預期同一前端
測試集合，不與前一輪累加。根目錄 identity pinning、setup 前及 cache 發布前
核對原 dev/inode、取消後 owned worker／選簿租約、狀態讀取與不可覆寫皆有後端測試。
重複設定、損毀身分及未知 GET 回應的拒絕屬自動化證據，不冒稱 Safari live。

交付資訊以持久交付根目錄內的 `ui4-verification/DELIVERY.json` 為準；
新 package 為 candidate3，其確切來源提交、雜湊與最終驗證紀錄由協調智慧體
填入外置 receipt，不在 repo 追寫自參照 hash。

## Safari 實際驗收

僅可拋棄資料及假測試金鑰，協調智慧體實際觀察：

- 明確開啟舊格式還原帳簿，報表精確顯示 `9,007,199,254,740,993`。
- 手動備份回報缺身分後，另勾獨立 checkbox 確認設定，成功且沒有自動備份或切簿。
  保持同一帳簿，再另行明確確認備份，成功獨立驗證、零附件。
- 中斷匯入：原生 picker 選擇工作上層、checkbox 明確確認，結果清理 **2**、
  已驗證目的地 **1**、進行中 **0**、未辨識 **1**。
- 中斷匯出：原生 picker 選擇來源帳簿與工作上層、checkbox 明確確認，結果清理 **1**、
  已驗證目的地 **0**、進行中 **0**、未辨識 **1**。
- Ctrl-C 後 fixture receipt 為 true，紀錄於
  `/private/tmp/kaccounting-ui4-recovery-safari.log`。五份保護檔案前後的
  dev／inode／length／SHA-256 皆相同：來源、已發布資料庫、來源附件及兩份
  未辨識內容保持保留；僅清理已認證且由本流程擁有的匯入 2／匯出 1 份工作。
- 可拋棄服務已停止，測試視窗已關閉。

這組 Safari fixture 使用已認證失敗工作／marker loss，不是 SIGKILL fixture；
不能把 UI 清理成功冒稱真實強制終止測試。

## 額外 Release 強制終止驗證

另以 Release 實跑真實 SIGKILL、128 MiB 中斷匯出復原測試，**1 項通過**，
log 為 `/private/tmp/kaccounting-recovery-sigkill.log`。
此為獨立後端程序驗收，不是 Safari 大檔案／實體跨 Mac 測試，
也不是把 routine verify 的其餘 ignored 項目全部重新驗證。

## 未改變的邊界

不自動切簿、覆寫目的地或重送未知寫入；不把 Downloads 交接當成下載完成。
正式簽章仍暫緩，永久私人代理、真實 Keychain、登入／重開機及實體跨 Mac／
大附件／外接媒體的待驗範圍不因本輪增量自動完成。前輪證據保留於
[附件與儲存紀錄](BUILD_STORAGE_UI_2026-10-23.md)。
