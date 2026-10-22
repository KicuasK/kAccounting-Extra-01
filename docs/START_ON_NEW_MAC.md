# 新電腦開始：kAccounting-Extra-01

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
4. `docs/VERSION01_DECISIONS.md`：已核准政策；後段包含最新四碼／視覺修訂。
5. `docs/CREDIT_CARD_CORE_REVIEW_2026-10-22.md`、`docs/OPEN_QUESTIONS.md`：未決政策。
6. `docs/DEVELOPMENT_PLAN.md`、`docs/API.md`、`docs/DATABASE_SCHEMA.md`。
7. `docs/NON_UI_REMAINING_2026-10-22.md`、`docs/BUILD_20MIN_2026-10-22.md`。

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

上一台結果：651 後端通過、9 刻意忽略，112 工具測試與 41 前端測試通過，
Release 成功；這只是交接基準，不能當成新機已通過。開發預覽僅依
`docs/LOCAL_RUNTIME.md` 使用新建拋棄式資料，不直接執行舊機暫存啟動器。

## 4. 下一輪工作

可以並行開始 UI 與非 UI 正式部署收尾。UI 現在仍只有唯讀科目樹；
13 張圖是設計，不是已實作產品。先做主視窗、明確選簿、科目頁，
再做分錄／報表／附件／備份與移轉。保持整數精確金額、後端原子平衡過帳、
已過帳不可變與稽核；分錄編輯的既有核准不重問。

四碼科目代碼須新增契約；信用卡額度與帳面餘額不可混用；未決分類、
正負號語意及信用卡政策不能自行假定。詳見未決文件。
正式私人代理、簽章與 Keychain、登入／重開機驗收仍待完成。
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
