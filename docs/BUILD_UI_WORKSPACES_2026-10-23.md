# 科目維護、分錄與報表開發紀錄 — 2026-10-23（台北）

本輪依使用者要求，以四個智慧體同時推進介面、非 UI 工具與交叉審查。
正式簽章暫緩，保留開發封裝；Version 01 尚未正式發行。

## 工作目錄與保留事項

- 工作目錄：`/Users/kicuashsu/.codex/worktrees/accounting-ui-handoff/kAccounting-Extra-01`。
- 分支：`codex/accounting-ui-handoff`，基準交接提交
  `171693561ffe64ff682074417923e11d4167bd78`；本輪與前輪成果尚未提交／推送。
- 原桌面目錄的 143 筆受追蹤未提交刪除保持原狀，未還原、覆寫或重設。
- 兩份 lockfile 保留；未重新生成整份 lockfile 規避問題。
- 科目代碼依使用者最新決定移除；帳戶餘額維持明確借／貸方向。
  信用卡可用額度、預繳、未償卡款等未決政策沒有擅自定案。

## 本輪完成範圍

- 科目維護：新增及編輯科目、完整路徑辨識、類別／過帳科目、啟用／停用、
  階層移動與後端版本核對。會計要素、階層、歷史與停用規則由既有後端執行。
- 分錄編輯：傳統日記簿表格、整數借／貸金額、科目路徑、摘要、明確儲存草稿、
  明確確認過帳、草稿刪除、已過帳唯讀、另建連結更正／沖銷分錄。
  不覆寫已過帳內容；過帳仍由後端原子驗證借貸平衡並保留稽核軌跡。
- 五張報表：普通日記簿、科目分類帳、試算表、資產負債表、收支損益表。
  明列截至日期或含起迄日的區間，分類帳以性質及完整科目路徑選擇，
  普通日記簿可開啟已過帳分錄。所有報表只取已過帳 API。
- 金額／識別／版本以精確十進位字串傳輸，計算使用 BigInt；核對日期、排序、
  重複識別、逐列流水餘額、報表合計與會計等式。不把缺失的金額欄位當成零。
  報表科目餘額顯示借／貸，損益摘要保留明確收益減費損語意。
- 工作區以不可變的帳簿與 selection generation 綁定請求；過期回應不採用，
  寫入中禁止切簿／切頁，未儲存輸入離開前確認。不自動重試未知結果的寫入。
  錯誤／版本衝突保留輸入，須先核對伺服器狀態再繼續。
- 介面沿用 02 緊湊全寬表格、底部摘要、黑白灰與五類科目色標。
  Vite build target 明確設定 `safari14`，這是輸出語法最低目標，
  不代表已在每個 Safari 14／macOS 組合完成實機驗收。
- 非 UI：停用私人部署計畫產生器及安全檢查／合成測試、來源改變時拒絕完成標記、
  簽章預檢與開發封裝持續保留。沒有安裝或啟動永久服務。

## 實際驗證

本輪完整 `./scripts/verify.sh` 成功，依 `/private/tmp/kaccounting-ui2-verify.log`
各 Rust 測試摘要加總：**654 通過、0 失敗、9 刻意忽略**。
同一輪另有 **132 Python 工具測試、104 前端測試通過**，TypeScript 與前端建置成功。
`cargo build --release --locked` 成功；紀錄位於
`/private/tmp/kaccounting-ui2-release.log`。完整 log 另已持久保存於下述交付根目錄的
`ui2-verification/*.log`；臨時 log 不應視為跨電腦可用的交付物。

協調智慧體使用 Safari 與可拋棄帳簿／假測試金鑰實測：新增科目、123 借貸草稿
明確儲存及確認過帳、已過帳畫面唯讀；普通日記簿、試算表、資產負債表、收支表
與分類帳顯示相符。`9007199254740993` 金額精確顯示，未經 Number 捨入。
這些是本機可拋棄環境的驗收，不代表真實 Keychain 或永久私人 HTTPS 部署通過。

後續 Safari 亦實測：分錄 2（ordinary，123 借貸）經明確確認產生分錄 3
（reversal、連結原分錄 2），借貸互換且唯讀；再從分錄 3 建立更正、填整筆說明並
明確確認，產生分錄 4（correction、連結原分錄 3），原已過帳分錄保持唯讀。
上述五報表的逐頁核對是在沖銷前進行。驗收後臨時伺服器已以 Ctrl-C 停止，
協調智慧體以 lsof 確認 3000 埠沒有 listener。

## 新開發封裝

持久交付根目錄：
`/Users/kicuashsu/.codex/visualizations/2026/10/22/01a1644d-21cf-7191-9321-1c50a73b82f5/deliverables/`

新 runtime 為 `kAccounting-Extra-01-development-ui2`，manifest SHA-256：
`82663f242e03f5572d5db7929d8f2a5314b09222b71c57652899a463df218dd5`。
舊開發封裝保留，沒有覆寫。此為 development runtime，不是正式簽署安裝程式。
新封裝亦獨立執行 verify 通過，結果 `complete=true`、`file_count=5`。

## 官方 Tailscale 固定來源重建

歷史暫存來源產物已不存在，本輪重新在私人持久目錄
`deliverables/tailscale-v1.104.2-kmgtcn1z/` 建置，目錄 0700，兩個 binary 0700。
官方 Go release metadata 驗證 `go1.27.3.darwin-arm64.tar.gz` 的 SHA-256：
`dd3908cf20959d798c1e06ebe5b72213f0505ce01373b40ce3110975a6cd99c2`。
使用獨立 GOPATH／GOCACHE／GOTMPDIR／GOBIN、`GOENV=off`、`GOTOOLCHAIN=local`，
沒有全域安裝或更改使用者 Go 設定。

Go checksum database 驗證 `tailscale.com v1.104.2`：
`h1:zBbSW0tI4Xi2PhrZEl9yt6oWay3l8A+M8scsTnZpyAk=`；官方 module origin 對應
`https://github.com/tailscale/tailscale` 的 `refs/tags/v1.104.2`、提交
`e26db7a5797683ec99ad1516c40b628c7f2dfd71`。
建置使用 darwin/arm64、CGO 關閉、trimpath 與上游支援的版本 linker stamps，
兩者僅執行 `--version`，回報 `1.104.2`、完整指定提交及
`1.104.2-te26db7a57`。因此本輪雜湊與歷史產物不同，沒有冒用歷史摘要。

| 產物 | 位元組 | SHA-256 |
| --- | ---: | --- |
| `bin/tailscale` | 23,939,378 | `61288a7ce502381b9a080c13f83fc5f3d235f9e6291336be7b3d46e7c56092cb` |
| `bin/tailscaled` | 34,918,962 | `d50d16f6d78ced0b767a73918aed27de3267d1dd5345241a56a896956200a700` |

此 staging 留有精確 schema 的 mode-600 `provenance.json`、詳細
`build-receipt.json`、官方 metadata、module download receipt、build command／log、
內嵌 build info 與版本輸出。部署產生器的 staged binary／provenance 合約檢查通過。
來源準備沒有啟動 daemon、登入、Serve、root 安裝、讀取 GUI 狀態或金鑰。

## 尚待完成

正式 Developer ID 簽章依使用者決定暫緩。永久私人代理安裝與生命週期、
實際 root Unix peer／私人 HTTPS、真實 Keychain ACL／提示、登入／登出與重開機
仍需專門驗收。編譯成功、停用計畫與合成測試均不構成這些項目的完成證據。
信用卡政策與完整產品驗收仍依現有待決文件追蹤。

前輪背景見 [接手開發紀錄](BUILD_TAKEOVER_2026-10-23.md)；
私人部署界線見 [私人部署準備](PRIVATE_DEPLOYMENT_PLAN_2026-10-23.md)。
