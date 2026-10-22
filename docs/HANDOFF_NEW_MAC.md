# kAccounting-Extra-01：換電腦建置與開發交接

交接日期：2026-10-22。GitHub：
<https://github.com/KicuasK/kAccounting-Extra-01>，交付分支 `main`。

## 先看這段

目前是**可以啟動的本機整合預覽，尚未完成 Version 01**。
記帳核心、私有 HTTP 執行環境、加密附件儲存、備份／還原與維護命令
已有實作與測試；前端仍只有 Catalina 風格的唯讀科目樹。
分錄編輯、完整報表與儲存操作介面尚未實作。

GitHub 包含原始碼、資料庫 migration、測試、文件、去識別化範例圖片及
Rust／npm lockfile。不包含正式帳簿、附件、備份、Keychain 金鑰、
`target`、`node_modules` 或 `dist`。新電腦重新編譯即可接續開發。

**這是原始碼交接，不是既有帳簿移轉。** Keychain 的裝置與收件者私鑰
不會經 GitHub 或自動同步搬到新 Mac。完整加密 `.kagextra` 匯出／匯入
仍未接完；本機備份也依賴原裝置金鑰。若另外存在正式帳簿，請保留原 Mac
及其金鑰，不能把 clone 成功視為財務資料已移轉。

## 新 Mac 的環境

已驗證環境是 Apple Silicon Mac（arm64）、macOS 26.6.2；
其他架構／macOS 版本需重新驗收，不宣稱都已測過。
「Catalina」指 UI 視覺方向，並非已驗證所有 Catalina 系統相容性。

| 工具 | 本次驗證版本／需求 |
| --- | --- |
| Git | 可 clone GitHub；上傳另需自己的 GitHub 權限 |
| Xcode 或 Command Line Tools | `xcode-select -p` 必須有有效路徑，供 Rust／SQLite 編譯 |
| Rust／Cargo | 1.99.1，`rust-toolchain.toml` 已固定版本，含 Clippy／rustfmt |
| Node.js | 24.22.1，`.nvmrc` 已記錄；套件最低要求為 22.12.0 |
| npm | 11.19.1，`frontend/package.json` 已記錄 |
| Python 3 | 操作／驗收輔助腳本使用標準函式庫 |
| zsh、rg | 驗證腳本需要；缺少 `rg` 時先安裝 ripgrep |
| Mac Safari | Version 01 目標瀏覽器；完整操作驗收尚待完成 |

先準備以上工具。缺少 Apple 開發工具時可執行 `xcode-select --install`。
Rust 使用 rustup 管理；Node 使用自己既有的版本管理方式即可。
SQLite 使用 bundled 版本，不必額外安裝資料庫伺服器。
建置與測試不需要 GitHub CLI、Tailscale 登入、應用密碼或真實帳簿。

## Clone、安裝相依套件、驗證

在新 Mac 選定原始碼目錄後執行：

```sh
git clone https://github.com/KicuasK/kAccounting-Extra-01.git
cd kAccounting-Extra-01
git switch main
git pull --ff-only
git rev-parse HEAD

rustup show
rustc --version
cargo --version
node --version
npm --version
python3 --version
rg --version
xcode-select -p

cargo fetch --locked
npm --prefix frontend ci
./scripts/verify.sh
cargo build --release --locked
```

若已安裝 nvm，可在根目錄使用 `nvm install`、`nvm use` 讀取 `.nvmrc`。
不使用 nvm 時，直接確認 Node 版本即可。首次 rustup 會按專案設定下載
1.99.1 工具鏈；首次 Cargo／npm 安裝需要網路。

使用 `npm ci` 與 `--locked`，不要為了開始建置先升級相依套件或改寫 lockfile。
如果無法取得下載來源，先處理新電腦的網路／權限，不要跳過驗證。

產物：後端 `target/release/kaccounting-backend`，前端 `frontend/dist/`。
新電腦自行產生這些檔案，不需要從舊電腦複製編譯快取。

## 只用測試資料啟動預覽

這個範例只建立隨機暫存帳簿與記憶體金鑰，不接觸使用者 Keychain，
無法指定既有帳簿。先確認 3000 埠沒有其他服務：

```sh
lsof -nP -iTCP:3000 -sTCP:LISTEN
cargo run --example local_acceptance --locked -- "$PWD/frontend/dist"
```

用 Safari 開啟 <http://127.0.0.1:3000/>，必須用這個確切位址；
`localhost:3000` 與其他 Host 會被拒絕。畫面會顯示兩個驗收科目。
這還不是完整可輸入分錄的介面。

終端會印出 `FIXTURE-TOKEN: …`。另開終端，切到同一專案，可執行：

```sh
python3 -B scripts/http-acceptance.py --fixture-token REPLACE_WITH_FIXTURE_TOKEN --writes
```

把 `REPLACE_WITH_FIXTURE_TOKEN` 換成剛印出的 32 位字串。
腳本確認對應的拋棄式帳簿後，驗證草稿、原子過帳、超過 JavaScript
安全整數範圍的金額、平衡報表和加密備份。這會修改該暫存帳簿；
只應搭配這個測試範例執行。完成後在服務終端按 Control-C，正常關閉並清理。

若開發環境的 sandbox 阻擋 loopback 監聽／連線，需要允許本機網路操作；
不要改成對外監聽，也不要為了測試啟用 Funnel。

## 額外檢查與預期結果

```sh
./scripts/verify-backend.sh
python3 -B scripts/test_launch_agent.py
cargo test --release --test attachments maximum_size_attachment --locked -- --ignored
git diff --check
```

交接前完整驗證結果：

- 472 個後端測試、41 個前端測試通過。
- 例行測試跳過的 250 MiB 容量案例已另外在 Release 模式通過。
- Clippy 將警告視為錯誤、格式、Rust 文件、TypeScript、Vite 建置通過。
- Release 後端可編譯；移轉附件的 Debug／Release 回歸通過。
- 3 個 LaunchAgent 產生器測試通過，既有檔案不會被覆寫。
- 實際 loopback HTTP 讀寫驗收通過，使用的都是虛構帳務與記憶體金鑰。
- 尚未驗收實際使用者 Keychain、登入啟動、完整 Safari 記帳流程。

測試數會隨後續功能新增而改變；以新電腦實際完整驗證結果為準。

## 明確建立新帳簿與操作命令

正式執行檔僅在明確呼叫 `init-new` 時建立新空白帳簿與本機 Keychain 項目。
`serve` 開啟既有帳簿時不會偷偷產生替代金鑰。命令、路徑、備份與
還原後身分設定，完整記錄於 [LOCAL_RUNTIME.md](LOCAL_RUNTIME.md)。

執行檔已提供：`init-new`、`serve`、`backup`、`restore`、
`setup-backup-identity`、`collect-attachments`、`recipient-request`。
帳簿在原始碼之外，預定根目錄為
`~/Library/Application Support/kAccounting-Extra-01/`。
`serve` 固定使用 `127.0.0.1:3000`。

LaunchAgent 產生器只產生 plist，尚未安裝或啟用登入服務。
Tailscale 身分／來源整合尚待完成，目前不要自行把 loopback API 對外發布。

## 已完成與程式位置

| 範圍 | 位置／狀態 |
| --- | --- |
| 科目與會計不變條件 | `backend/src/accounting/`，精確整數、平衡過帳、階層與啟用規則、草稿 revision、沖銷／更正 |
| 報表 | `accounting/reports.rs`，來自已過帳明細的日記簿、總帳、試算表、資產負債表與收支 |
| 資料庫 | `backend/src/db.rs`、`backend/migrations/0001`–`0007`，完整性稽核、交易與觸發器 |
| HTTP | `api.rs`、`api/mutations.rs`、`api/book_commands.rs`，精確字串整數、來源保護、附件 metadata／驗證／草稿移除與備份 |
| 啟動／命令 | `runtime.rs`、`application_commands.rs`、`main.rs`，明確帳簿、金鑰綁定與互斥鎖 |
| 加密附件 | `attachments.rs`、`attachment_maintenance.rs`，250 MiB／檔、20 GiB／帳簿、StorePin 與離線清理 |
| 金鑰／備份 | `keys.rs`、`book_backup.rs`、`backup_service.rs`，Keychain adapter、完整驗證、新目的地還原、同帳簿最新 10 份管理備份 |
| 移轉基礎 | `crypto.rs`、`recipient_transfer.rs`、`encrypted_transfer.rs`，串流容器、HPKE、附件重新加密；不等於完整套件流程 |
| 前端 | `frontend/src/`，唯讀科目樹、請求生命週期與 Catalina 風格；完整 UI 尚未做 |

## 接手的優先順序

依現有要求，先完成非 UI 功能，再做剩餘 UI：

1. 完成加密 `.kagextra` manifest、資料庫與全部附件的參照核對、封裝、
   發布、匯入及持久化中斷復原。舊 `transfer.rs` 有未加密結構性基礎，
   不可把它直接宣稱為已核准的加密跨 Mac 流程。
2. 接上附件串流上傳／下載與剩餘還原／移轉 HTTP 命令。
   不得使用 250 MiB 明文緩衝或明文暫存檔代替有界串流。
3. 完成舊 schema 遷移前的加密備份；目前舊 schema 會安全拒絕啟動。
   補齊 active-book 設定、巢狀管理備份暫存復原與狀態呈現。
4. 驗收真實 Keychain 的鎖定／登入行為、可移除磁碟檔案系統、
   代表性大型帳簿、安裝及私人 Tailscale 身分／來源整合。
5. 實作科目管理、General Journal 編輯、報表、附件、備份及移轉 UI，
   最後完成 Mac Safari 端到端驗收。

General Journal 的詳細欄位與明確 Save 互動已於 2026-10-22 核准，
見 [VERSION01_DECISIONS.md](VERSION01_DECISIONS.md)；不要再次要求同一核准。
手寫科目預設樹與分類語意仍有未決事項，見
[OPEN_QUESTIONS.md](OPEN_QUESTIONS.md)，不可自行猜測並植入正式預設資料。

高容量附件是 Version 01 範圍。iPad 觸控 UI、CSV、TPASS 都不屬於
這次 Version 01 完成範圍。`TPASS_EXAMPLE_PACKAGE` 是另案 Swift 專案的
未來功能參考，其 verifier 不是本 Rust 專案的驗收入口。

## 給下一個 Codex 的交接提示

```text
請接手 kAccounting-Extra-01。先讀 AGENTS.md、docs/HANDOFF_NEW_MAC.md、
docs/VERSION01_DECISIONS.md、docs/DEVELOPMENT_PLAN.md、docs/API.md。
先執行 npm --prefix frontend ci 與 ./scripts/verify.sh，記錄 HEAD 和結果。
目前是可啟動的整合預覽，前端仍只有唯讀科目樹，不能宣稱 Version 01 完成。
按交接文件優先順序，先補完整加密跨 Mac 套件與附件串流等非 UI 閉環，再做 UI。
維持精確整數金額、交易式平衡過帳、已過帳不可變與來源分離。
不要接觸真實帳簿，不要把帳簿／金鑰／備份提交到 Git，不要啟用公開服務。
已核准的政策不要重新詢問；模糊的手寫科目分類不要自行決定。
每個可驗證增量報告修改檔案、測試結果、未完成項目與 Git 狀態。
```

## 其他交接資料

- [本機操作指南](LOCAL_RUNTIME.md)
- [一小時實作紀錄](BUILD_SESSION_2026-10-22.md)
- [六個 Codex 整合紀錄](PARALLEL_BUILD_2026-10-22.md)
- [架構](ARCHITECTURE.md)、[資料庫](DATABASE_SCHEMA.md)、[API](API.md)
- [開發計畫](DEVELOPMENT_PLAN.md)、[UI 規格](UI_STYLE_GUIDE.md)

先前紀錄的「未提交」是各場次當下狀態。本次發布以 GitHub `main` 的
實際提交 SHA 為準；新電腦用 `git rev-parse HEAD` 記錄接手版本。
