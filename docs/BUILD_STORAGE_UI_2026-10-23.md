# 附件、儲存與原生本機介面開發紀錄 — 2026-10-23（台北）

本輪依使用者授權，以四個智慧體並行完成附件、儲存介面、原生本機橋接及
封裝／部署檢查。前輪的科目維護、分錄編輯與五報表持續保留；
Version 01 仍未正式發行，正式簽章依決定暫緩。

## 已實作

- 附件 UI：分錄層級附件選擇、明確確認後上傳、清單、精確位元組、驗證、
  Safari 原生下載；草稿可明確移除參照，已過帳僅追加，沒有替換／移除入口。
  草稿操作攜帶目前版本並重新核對分錄，未儲存分錄或未知附件結果阻擋重複寫入。
- 儲存 UI 八項流程：手動加密備份、收件請求、匯出、資料夾包匯入、`.kagextra`
  單檔匯入、備份還原、中斷匯入復原、中斷匯出復原。明確確認／完整指紋核對、
  不定進度與工作狀態、保留清理警告、未知結果不重送。還原／匯入只建立新目的地，
  不覆寫、不自動切簿；中斷復原重新選擇工作上層，未辨識內容保持保留。
- 本機原生介面：八種有限 kind 的真實 macOS 選擇與新建空白帳簿。
  不接受網頁路徑、金鑰、密碼、overwrite 或任意 capability 註冊；
  強制 Origin／selection generation，私人 Unix 入口不註冊本機面板路由。
  新建帳簿必須是原生選定的集合內全新 direct-child，建立後仍由使用者明確選簿。
- 下載：原生 Safari attachment／匯出連結以 generation query 保護，拒絕舊帳簿
  的下載。附件識別使用真實 32 字元 lowercase hex；下載交接訊息不冒稱寫檔完成。
  匯出政策維持先 Downloads，再由使用者手動移動。
- 上傳：附件與加密移轉單檔使用原始 File body 與既有串流端點，不轉 JSON／base64，
  不加入瀏覽器端整檔 ArrayBuffer／Blob 複製。大小、ownership、版本與內容驗證仍由
  後端執行。金額／識別／計數保留精確十進位字串，不經浮點數。
- 非 UI：完整 `StoragePicker.app` helper 封裝及其 Info.plist／binary 摘要檢查、
  保留 bare-helper 相容性、部署 review 離線 verify 與來源 identity／hash 綁定。
  新封裝／review 均不安裝或啟動永久服務。

原生新帳簿面板另改善可信初始目錄，使集合位置由受信任主機設定，
不由 browser 輸入。改善後完整重驗及 Safari 新帳簿原生流程均已通過。

## 最終自動化驗證

可信初始目錄改善後，最後一輪完整 `./scripts/verify.sh` 通過：
**664 Rust 通過、0 失敗、9 刻意忽略；141 Python、132 前端通過**，
TypeScript／前端 production build 亦成功。
候選重驗來源為 `/private/tmp/kaccounting-ui3-release-candidate-verify.log`；
`/private/tmp/kaccounting-ui3-release.log` 顯示 `cargo build --release --locked` 成功。
兩份 lockfile 沒有重新生成。

前一 checkpoint 為 137 Python，其餘數字相同；不可與本輪數字累加。
後續初始目錄修正的 checkpoint 為 138 Python；最後封裝 startup 修正新增三項測試，
工具數為 141。最終候選完成狀態與持久 log 以外置 `ui3-verification/DELIVERY.json` 為準。
儲存單元另實跑 17 項 targeted 測試（10 模型、7 模擬 DOM）及 TypeScript 通過，
包括未知提交不重試、明確轉復原、錯誤 job ID 不假成功及完整確認流程。

## Safari 實際證據與限制

僅使用可拋棄帳簿、fixture 檔案與假測試金鑰，沒有真實財務帳簿／既有金鑰。

- 手動備份：checkbox 與明確確認後成功建立已驗證管理備份，附件數／位元組／
  保留未辨識項目皆為零。
- 已過帳分錄附件：選擇檔案不會立即上傳；明確確認後成功追加 62-byte
  `kAccounting-ui3-receipt.txt`，清單與內容完整性驗證通過，未提供移除入口。
- Safari Downloads 實際完成 62-byte 下載，來源及下載檔案 SHA-256 均為
  `2b22672456be7d47674f3d739c627655a6b28cf1311e787075eab808621dc809`。
- 新建帳簿：可拋棄 fixture `06e9107a4a65c6b0b6da2d55201a2b4d` 的原生面板
  初始集合位置正確，填入 `Safari-New-Book` 並按 Save 後，UI 顯示空白帳簿已建立、
  尚未選簿，沒有自動開啟。先前全路徑 Save As 被拒絕，已以可信初始目錄修正並重驗。
- 同一 Mac 兩個邏輯端的收件請求 → 匯出 → Downloads → 單檔匯入 → 明確選簿
  → 大整數試算表鏈已實測通過，細節如下。備份還原亦成功；資料夾包匯入及中斷復原的
  Safari 操作仍待驗收。合成 picker、模型或 route 測試不替代實體跨 Mac／
  真實 Keychain／大附件移轉驗收。

### Safari 儲存正向鏈

此鏈使用可拋棄 fixture `06e9107a4a65c6b0b6da2d55201a2b4d`、假測試金鑰，
在同一 Mac 模擬兩個邏輯端。來源根目錄是 `std::env::temp_dir()` 的 canonical
`/private/var/folders/.../T/ka-http...`，不是 `/private/tmp`。
先前錯誤 root／全路徑選擇被拒絕，不計入成功證據；修正後使用正確原生目錄操作。

1. 來源帳簿已明確選擇；原生 RequestDestination 儲存 `recipient-request.json`，
   確認工作成功，得到公開請求識別 `71ad75a89cc4797c418ac9fa703aefd3`、公鑰指紋
   `967041c11e04559bc221d10ea5457e02a1cd7aa68a06267a01e22699afe7ce94`。
2. 原生選擇來源帳簿、同一 request、新 `transfer.kagextra` 目的地；畫面完整指紋
   與前項一致，checkbox 明確確認後，工作 `1e01a422efb1fd949c79fa8e5fac5f22`
   成功並驗證 package `cfbec3d9808e476413b186f4f05f1232`，附件數與位元組皆為零。
3. Safari Downloads 實際完成 `kAccounting-Extra-01.kagextra`（Safari 顯示約 112 KB）。
4. 選擇單檔匯入、同一 request、新 `Safari-Imported-Book` 帳簿目的地，並以
   browser File 選取上述 Downloads 檔案。明確確認後顯示已驗證單檔匯入成功，
   package ID 與前項相同、零附件；來源帳簿仍保持選擇，沒有自動切簿。
5. 更新候選清單後，由使用者明確選擇 `Safari-Imported-Book` 並開啟。
   試算表借方及貸方各精確顯示 `9,007,199,254,740,993`。

### Safari 備份還原

同一 fixture 的來源帳簿明確建立並驗證管理備份：
`8ac106cd9484e17367799dcde48decf6-000000000000000000001792722686112956000-605e6afa8cbdb12aeda19edb78e77c76.managed`。
原生選擇該目錄內的 `backup.kabackup`（不是外層 `.managed` 目錄）與集合內全新
`Safari-Restored-Book` 目的地。checkbox 明確確認後，工作
`698027988cdcc806533836cdee4734de` 成功，附件數及位元組皆為零，
`selection_required=true`，UI 仍保持來源帳簿選擇，未自動開啟還原帳簿。
本輪未另外開啟還原帳簿核對報表，不能聲稱該步驟已完成。
UI 已補「加密備份包（backup.kabackup）」與不選外層 managed 的說明。

上述結果不代表實體跨 Mac、大附件 transfer、資料夾包匯入或中斷復原
已驗收；這些項目仍分別保留待驗。

詳細逐項驗收見 [儲存及附件驗收](STORAGE_UI_ACCEPTANCE_2026-10-23.md)，
儲存單元合約見 [儲存工作區](STORAGE_UI_WORKSPACE_2026-10-23.md)。

## 持久產物與 Git

持久交付根目錄：
`/Users/kicuashsu/.codex/visualizations/2026/10/22/01a1644d-21cf-7191-9321-1c50a73b82f5/deliverables/`

- `StoragePicker-ui3.app`：本輪完整 helper bundle。
- `kAccounting-Extra-01-development-ui3`：新 runtime，獨立 verify 通過，
  `complete=true`、6 files；包含 `native/StoragePicker.app` 與 UI3B helper。
  此為保留的較早 ui3 包；最新候選產物及來源提交的確切綁定，統一以外置
  `ui3-verification/DELIVERY.json` 為準，不把會隨文件修改改變的最終 hash 追寫入 repo。
- `kAccounting-Extra-01-development` 與 `kAccounting-Extra-01-development-ui2`：舊開發封裝保留。
- `tailscale-v1.104.2-kmgtcn1z`：前輪官方固定來源 staging、provenance／build receipt。
- `private-deployment-EXAMPLE-ONLY-rcvg48v_` 與相鄰 receipt：僅停用範例 review，
  不是永久安裝或 production qualification。

工作分支仍為 `codex/accounting-ui-handoff`，managed worktree 位於
`/Users/kicuashsu/.codex/worktrees/accounting-ui-handoff/kAccounting-Extra-01`。
原 Desktop checkout 的 143 筆未提交受追蹤刪除未碰、未還原、未強制重設。
前輪完整來源已保存為本機提交 `f9b50ba`；封裝 startup 修正與本文件更新另由
協調智慧體保存小提交，實際最終提交以分支 Git 紀錄及外置 DELIVERY 為準。
財務資料、金鑰與產物不加入 Git，遠端發布仍另行處理。

額外容量驗證：`api_attachment_streaming` 的
`maximum_http_stream_roundtrips_and_undeclared_oversize_cleans_candidate`
以 Release 明確執行 `--ignored` 並通過，涵蓋最大 HTTP 附件來回及未宣告超額清理。
這是 routine verifier 的九項忽略之外另跑的一項，不代表另外八項重跑。
該 Release 容量測試實際為一項通過，不是 Safari 大檔案實測。
最終 development runtime 與來源提交的綁定，記錄於交付目錄內的
`ui3-verification/DELIVERY.json`。
帶 native helper 的新包必須使用 `serve-native-chooser` 及 exact helper argument；
已新增三項封裝回歸測試。舊包仍可驗證但明示 `legacy_native_startup`，
不能將較早 `serve-chooser` manifest 當成完整原生啟動指令。
Safari 拋棄式服務與其原生面板已停止，3000 埠確認沒有 listener。

## 剩餘界線

永久私人代理與 reboot 生命週期、正式封裝／Developer ID 簽章、真實 Keychain ACL／
授權提示與登入／登出／重開機仍待，沒有因新增按鈕或產生停用計畫就完成。
本輪未安裝永久服務、未簽署正式版、未使用真實財務金鑰。
私人 HTTPS 必須保留 root Unix peer 與身分核對，不能用普通 TCP 或 GUI 既有狀態替代。

信用卡餘額、可用額度、預繳款、未償卡款與政策仍分開追蹤，未以 UI 實作擅自定案。
一般餘額維持使用者決定的借／貸方向，不重新加入已移除的顯示科目代碼。
實體 APFS／exFAT 外接媒體及移除／中斷驗收依使用者決定留待 Version 02。

前輪成果與證據：[科目／分錄／五報表](BUILD_UI_WORKSPACES_2026-10-23.md)。
