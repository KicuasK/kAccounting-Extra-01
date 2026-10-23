# 私人代理版本升級與回復工具

本輪只開發單台 Mac 主機的非 UI 管理工具，使用隔離檔案與模擬 macOS 系統邊界驗證。未執行管理員安裝、真實代理升級／回復、Tailscale 登入、Keychain、登入或重開機驗收；目前試用服務保留。沒有前端建置、簽章或公證。

## 版本操作的邊界

`scripts/update-private-transport.py` 提供獨立的 `upgrade`、`rollback` 與 `reconcile` 操作。預設唯讀 plan，實際變更須明確 `--apply`，而且只允許 macOS root 的 isolated Python。固定的私人代理工作必須不存在、control socket 必須不存在；工具不會停止服務、刪除 socket 或自動開始新版服務。

版本切換保留舊 binaries、安裝 receipt、操作意圖與結果。代理 state 不做轉換或還原，帳簿與金鑰也不在此工具的變更範圍內。切換時重新核對 receipt、檔案身分與雜湊、state，以及排他鎖；中斷後需明確核對，不自動覆寫未知檔案或重新執行啟停。

回復只允許目前版本對應的上一筆完成升級，而且新版 guard 尚未嘗試執行 daemon、state 仍與升級時相同。guard 在執行前留下使用標記；即使啟動失敗，也不把它當作可安全降版。新版一旦使用或 state 改變，需另外評估版本相容性／移轉，不能把 binaries 回復當成 state 復原。

舊 guard 沒有 `UPGRADE_PROTOCOL` 時拒絕升級，必須另做可審查的移轉方案。本機尚無真實 root 安裝；新的安裝工具包會採用支援此協定的 guard，因此這是現有部署前的保護，並非宣稱可接管任意舊安裝。

## 操作者資料與驗證界線

新版本來源需獨立審查及外置 SHA-256 綁定；版本與來源聲明只是操作者審查資料，不能取代供應商簽章或執行相容性驗證。實際新代理執行檔仍需由操作者取得並核對，工具不自動下載或登入。

本輪可審查工具包為 `deliverables/private-operations-kit2`；最終來源綁定、非 UI 驗證與工具包雜湊記錄於外置 `deliverables/private-transport-version-verification/DELIVERY.json`。既有 candidate6 保留，沒有重新建置 UI。正式使用前仍需實際安裝、私人路由、Keychain、登入／重開機及服務健康驗收；隔離測試不代表已上線。

## CLI 與審查檔

以下為介面示意；`H_INSTALL`、`H_REVIEW`、`ABS_REVIEW`、`UPGRADE_ID` 必須替換為獨立核對的實際值，不能直接執行。正式 root 操作前需操作者授權。固定安裝位於 root 私有目錄，因此即使 plan 為唯讀，讀取已安裝資料仍需要相應權限。

```text
python3 -I -S scripts/update-private-transport.py upgrade --expected-install-receipt-sha256 H_INSTALL --review ABS_REVIEW --expected-review-sha256 H_REVIEW --plan
python3 -I -S scripts/update-private-transport.py rollback --expected-install-receipt-sha256 H_INSTALL --upgrade-id UPGRADE_ID --plan
python3 -I -S scripts/update-private-transport.py reconcile --expected-install-receipt-sha256 H_INSTALL --plan
```

只有明確改為 `--apply` 才會變更；`--plan` 與 `--apply` 互斥。回復的 upgrade ID 是目前版本對應之已完成升級紀錄的 32 位小寫十六進位 ID。reconcile 接受 pending 記錄之前或之後的 receipt hash，只核對並完成已記錄的檔案切換，不啟動服務、不轉換 state，也不清除未知殘留。

升級 review 檔為 mode0600 的 JSON，`format` 為 `kAccounting-Extra-01.private-transport-version-review.v1`。`provenance` 含 `version`（三段數字，如 `1.2.3`）、`source_url`（固定 `https://github.com/tailscale/tailscale`）、`source_revision`（40 位小寫十六進位提交）及 `build_note`（1–2000 字元）。`transport` 下的 `tailscale`、`tailscaled` 各含絕對 `path` 與 `sha256`。來源 binaries 需為 mode0700、與已安裝 thin 64-bit Mach-O CPU 類型相同；工具不執行來源 binaries 驗證版本。guard 保持已安裝且受 hash 固定的版本，不接受 review 替換 guard。

plan 只檢查可讀輸入並列出前置條件，不能當作實際停止與切換完成證明。apply 另取得排他鎖、確認精確 launchctl 工作不存在、control 目錄不存在或安全且空白、state 仍未變，再執行。任何不確定狀態均保留檔案並拒絕繼續；先檢查實際狀態，再使用明確 reconcile。

升級 plan 的 proposed receipt 含新產生的 UUID，因此不同次 plan／apply 的 proposed receipt SHA 可能不同；它不是操作者預先固定的授權值。授權核對的是目前 install receipt、review 及兩支 binaries 的 SHA-256。真正 apply 完成後，必須將輸出的新 install receipt SHA 獨立保存，供後續啟停或版本操作核對。

state 核對包括檔案身分、連結／類型與讀取期間的變化；回復在 staging 後仍綁定原升級紀錄的 state hash，避免用中途變化後的 state 建立新的回復基準。這些為隔離測試保護邊界，不表示已在正式代理上執行升級或回復。

## 凍結後非 UI 驗證

最終 `verify-backend.sh`：684 個 Rust 測試通過、0 失敗、9 個刻意忽略；256 個工具測試通過。工具基線為 221 個，本輪增加 29 個 updater、4 個 guard、2 個 manager 測試，共 35 個。格式、Clippy 與 Rust 文件檢查通過，locked backend Release 建置通過；後端產品來源與 lockfile 未改。未執行前端／UI 建置。

最終紀錄為 `kaccounting-transport-version-final-fixed.log`；較早的 baseline／final 紀錄保留作開發歷史，不取代凍結後結果。升級來源審查、停止與 state 條件、未使用新版回復、staging／promotion 中斷、回復鏈與 archive 綁定皆使用隔離檔案及模擬系統邊界測試；沒有操作真正 root 安裝或既有 Tailscale state。

plan 的 `target_receipt_role` 在升級時為 `provisional-generation`，在回復時為 `retained-exact-receipt`；請配合前述 receipt hash 保存規則判讀，勿將升級 plan 的暫定 hash 當成 apply 結果。
