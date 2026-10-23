# 新電腦開始：kAccounting-Extra-01

最新非 UI 增量：[私人代理版本升級與回復工具](PRIVATE_TRANSPORT_VERSIONING_2026-10-23.md)；只完成隔離驗證，未套用真實管理員安裝，試用服務保留。

最新非 UI 工具：[安裝、啟停、更新與復原](PRIVATE_RUNTIME_OPERATIONS_2026-10-23.md) 已完成隔離測試；未套用系統安裝，含 root 工作控制與選填登入啟動設定，但實際安裝、私人連線及登入／重開機驗收仍待完成。試用服務保留。

本輪非 UI 工具與驗證結果：[單主機非 UI 增量](SINGLE_HOST_NON_UI_2026-10-23.md)。

## 目前部署範圍 — 2026-10-23（優先於下方歷史）

使用者已決定採 **單台 Mac 主機＋私人遠端瀏覽器**：帳簿與後端留在這台 Mac，
其他裝置透過具身分驗證的私人 HTTPS 操作；不公開服務、不啟用 Tailscale Funnel。
**Developer ID 簽章與公證不做**，不再列為部署前置條件。實體跨 Mac 驗收改列
備援／未來工作，不是此單主機上線門檻；加密備份與還原仍須保留。

本輪不改 UI、不執行前端／UI 建置。最新已驗證開發封裝是 **candidate6**，含無 PR
分錄版面，使用既有資產；來源與封裝證據見外置
`deliverables/candidate6-verification/DELIVERY.json`。
使用者目前操作的 candidate6 前端＋假資料測試頁仍在運行，**不要停止、取代該服務
或占用其連接埠**。這不是正式常駐服務，也不能視為真實 Keychain 驗收。

下一步從短且可獨立完成的非 UI 工作開始：

1. 用可拋棄資料驗證既有部署計畫、封裝完整性及工具拒絕案例，保留封裝與 lockfile。
2. 補齊可審閱的安裝／啟停／更新／復原工具及操作指引，保留既有程式、資料與
   未知內容；先做離線驗證，不把產生計畫當成已安裝。
3. 只跑後端／工具驗證與 locked Release，另列本輪實際結果，不沿用歷史完整測試數。
4. 正式私人代理身分與設定、真實 Keychain、常駐啟動、登出／登入和重開機仍待
   各自驗收；需要使用者授權或操作 Mac 時，再說明具體動作。尚未宣稱常駐服務
   已安裝、啟用或通過正式驗收。

下方 UI、簽章、跨 Mac 門檻及工時為歷史紀錄，由本節取代目前執行順序。

## 最新增量 — 2026-10-23 草稿附件與資料夾移轉包 Safari 驗收

草稿附件原生取消、選檔後未上傳的儲存保護、確認取消、上傳、完整性驗證、
移除及未儲存文字保護，均已用可拋棄 Safari fixture 驗收。資料夾移轉包經真正
原生選擇與獨立確認匯入，維持目前選簿；另行開啟新帳簿後，試算借貸各精確為
`9,007,199,254,740,993`，附件驗證通過，來源／套件／請求檔保持不變。
詳見 [本輪驗收](DRAFT_FOLDER_SAFARI_2026-10-23.md)。完整驗證為
**684 後端、141 工具、185 前端通過，9 項刻意忽略，0 失敗**。
本輪只有驗收 fixture 與文件變更，正式程式、前端與 lockfile 未變；
保留已驗證的 candidate5 開發封裝，不重包相同正式程式。外置證據為
`deliverables/draft-folder-verification/DELIVERY.json`。
正式簽章仍暫緩；正式私人代理、真實 Keychain／登入／重開機及實體跨 Mac
仍待驗收，本機假金鑰驗收不能取代。下方 checkpoint 保留歷史。

## 前次增量 — 2026-10-23 部分付款與 AR/AP 拆分／篩選（保留歷史）

付款備忘錄已可輸入本次部分付款金額，預設剩餘待付，建立草稿後仍須另行過帳；
AR/AP 已可將來源整行拆成多筆款項，並依類型、狀態與事項／對象篩選。
詳見 [本輪實作與驗收](PARTIAL_SPLIT_UI_2026-10-23.md)。完整驗證實際為
**684 後端、141 工具、185 前端通過，9 項刻意忽略；locked Release 成功**。
Safari 通過 1,000 TWD 計畫分 400＋600 付款、AR 500 拆 300＋200、
AP 300 拆 100＋200、篩選與 75 TWD AP 付款分配。
candidate5 開發封裝來源、雜湊與證據以外置
`deliverables/partial-split-verification/DELIVERY.json` 為準。
正式簽章仍暫緩；私人代理、真實 Keychain／登入／重開機及實體跨 Mac 仍待驗收。
本輪未完成信用卡、TPASS、帳齡報表或背景通知。既有 migration 8–10 升級保護維持。

## 前次增量 — 2026-10-23 付款備忘錄與 AR/AP（保留歷史）

已新增未來付款備忘錄與 AR/AP 逐筆追蹤，詳見
[本輪實作與驗證](PAYMENT_ARAP_2026-10-23.md)。本輪實際完整驗證為
**684 後端、141 工具、171 前端通過，9 項刻意忽略；locked Release 成功**。
candidate4 開發封裝的來源綁定、雜湊及證據以外置
`deliverables/payment-arap-verification/DELIVERY.json` 為準，不以舊包替代。

新增 migration 8–10。開啟既有帳簿時，必須保留原 runtime 的受保護升級／
migration backup 流程；先備份、驗證後升級，不手動改 schema 或跳過保護。
舊 binary 不可讀取已升級的新 schema，也不可為降版移除 migration 紀錄。
正式簽章仍暫緩。取得新 Mac 原始碼前須核對實際 Git 歷史與本輪來源綁定。

## 前一輪基準 — UI4（保留歷史）

前一輪基準為 UI4 提交 `35d6d0f35aea2962f1322967959d559cf09d1294`：
**669 後端、141 工具、141 前端通過，9 項刻意忽略，locked Release 成功**。
舊格式還原後備份身分設定、還原帳簿科目餘額及中斷復原 Safari 驗收已通過；
另有獨立 Release 128 MiB SIGKILL 復原測試通過。詳見
[UI4 證據](BACKUP_IDENTITY_RECOVERY_2026-10-23.md) 及
[目前交接](HANDOFF_NEXT_MAC.md)。candidate3 開發封裝的來源綁定與雜湊見外置
`deliverables/ui4-verification/DELIVERY.json`。此提交尚未推送，遠端 main 未必包含
最新成果，接手前必須核對，不可把 clone 成功當作已取得本輪程式碼。
使用者已取消科目代碼，餘額顯示借／貸；正式簽章暫緩，保留開發封裝。

本文件與 UI 示意圖直接隨 GitHub 原始碼提供，不另製作交接包。
專案：<https://github.com/KicuasK/kAccounting-Extra-01>。

## 1. 取得專案

建議 clone 到 `~/Desktop/kAccounting-Extra-01`，保留完整 Git 歷史：

```sh
cd ~/Desktop
git clone https://github.com/KicuasK/kAccounting-Extra-01.git
cd kAccounting-Extra-01
git status --short
git branch --show-current
git log -1 --oneline
```

若資料夾已存在，先檢查狀態並保留未提交修改，不重新 clone 覆蓋、不 reset。
確認乾淨後才能以 `git pull --ff-only` 更新；遇到分歧先評估，不強制覆寫。
GitHub 登入由新電腦自行完成，不複製舊電腦的登入憑證。

也可在 GitHub 按 Code → Download ZIP。ZIP 含程式碼、文件與圖，但沒有 `.git`，
下一輪要提交開發成果時仍建議使用 clone，不把 ZIP 誤認為完整 Git checkout。

## 2. 接手先讀

1. `AGENTS.md`：安全、會計與最新 UI 約束。
2. `docs/HANDOFF_NEXT_MAC.md`：實測成果、環境、正式部署缺口與操作政策。
3. `docs/UI_DESIGN_HANDOFF_2026-10-22.md`：另一個對話的設計要求、取代順序、原圖。
4. `docs/VERSION01_DECISIONS.md`：已核准政策；以最新取消代碼／借貸方向決策為準。
5. `docs/CREDIT_CARD_CORE_REVIEW_2026-10-22.md`、`docs/OPEN_QUESTIONS.md`：未決政策。
6. `docs/DEVELOPMENT_PLAN.md`、`docs/API.md`、`docs/DATABASE_SCHEMA.md`。
7. `docs/BACKUP_IDENTITY_RECOVERY_2026-10-23.md`、`docs/BUILD_STORAGE_UI_2026-10-23.md`。
8. `docs/NON_UI_REMAINING_2026-10-22.md`、`docs/BUILD_20MIN_2026-10-22.md`：歷史背景。

舊建置報告保留歷史，不覆蓋最新交接與使用者決策。

## 3. 準備環境並重新驗證

已驗證主機為 Apple Silicon/macOS 27.0；新電腦必須自行驗證，不宣稱所有 Mac
或 Catalina 系統版本均相容。Catalina 原本是外觀參考，最新設計不限該版本。
需 Xcode Command Line Tools、Rust/Cargo、Node/npm、Python 3、zsh、ripgrep。
上一台使用 Rust 1.99.1、Node 24.22.1、npm 11.19.1。

工具需在 PATH，先查 `rustup show`、`cargo --version`、`node --version`、
`npm --version`、`python3 --version`、`xcode-select -p`。接著在專案根目錄：

```sh
cargo fetch --locked
npm --prefix frontend ci
./scripts/verify.sh
cargo build --release --locked
```

保留 `Cargo.lock` 及 `frontend/package-lock.json`。鎖定依賴含 yanked 版本，
不要重新生成整份 lockfile。新電腦若無法取得或版本不同，先診斷原因。
上述建置不需要實際帳簿、Tailscale、管理員代理或財務 Keychain。

上一台最新結果為上方 UI4 的 669／141／141、9 忽略與 Release 成功；
這只是交接基準，不能當成新機已通過。開發預覽僅依
`docs/LOCAL_RUNTIME.md` 使用新建拋棄式資料，不直接執行舊機暫存啟動器。

## 4. 下一輪工作

科目維護、分錄草稿／過帳／沖銷／更正、五報表、附件與八項儲存流程已實作；
13 張原圖仍是設計參考，不可代替實際產品驗收。先從可自行完成的短工作補齊
原生取消、目錄套件匯入與代表性大附件 Safari 驗收，使用可拋棄資料與假金鑰。
每次變更後執行適當檢查，整合完成再跑完整驗證、locked Release 與開發封裝。
保持整數精確金額、後端原子平衡過帳、已過帳不可變與稽核；既有核准不重問。

信用卡額度、預繳款、未償卡款與帳面餘額不可混用；未決分類及信用卡政策
不能自行假定。正式私人代理、真實 Keychain／登入／重開機與實體跨 Mac
仍待驗收，需要授權或操作 Mac 時再告知使用者。正式簽章暫緩。
實體 APFS/exFAT 驗收與 iPad UI 在 Version 02，保留既有碼，不阻擋 UI 開始。

## 5. 這次沒有搬的東西

不包含真實帳簿、資料庫、附件物件、備份、加密套件、Keychain 金鑰、
GitHub/Tailscale 登入狀態、私人設定、已編譯產物或依賴目錄。
舊機 `/private/tmp` 中的工具／runtime 不是可在新機照貼的交付路徑。
GitHub 同步不是財務資料移轉；不要為了預覽找出或讀取真實帳簿。
不啟用公開服務或 Tailscale Funnel。需要 OS 授權或使用者操作時再詢問。

## 可貼給下一個開發對話的指示

> 接手桌面的 kAccounting-Extra-01；若不存在，從 KicuasK/kAccounting-Extra-01
> clone 最新 main，已存在先保留未提交修改。先讀 AGENTS.md、
> docs/START_ON_NEW_MAC.md、docs/HANDOFF_NEXT_MAC.md、
> docs/UI_DESIGN_HANDOFF_2026-10-22.md、VERSION01_DECISIONS、API、
> DEVELOPMENT_PLAN、OPEN_QUESTIONS 與信用卡核心評估。
> 準備環境、執行完整驗證與 Release，再續作 UI 與非 UI 收尾。
> UI 採最新 02 結構、黑白灰且僅五類用色；原圖是示意，文字最新要求優先。
> 技術問題自行決定，未決會計政策與需要我操作的事再問我。
> 不碰真實帳簿、不提交金鑰／資料庫／備份、不啟用公開服務或 Funnel。
> 定期回報修改、測試、剩餘工作與 Git 狀態。
