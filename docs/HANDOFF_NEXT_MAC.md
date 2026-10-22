# 下一台 Mac 開發交接（2026-10-22）

## 交付狀態：使用者已授權提交並上傳 GitHub

2026-10-22 使用者明確指示「上傳 GitHub」，取代先前暫不上傳的要求。
本次交付包含至 22:26 建置輪次的本機原始碼、測試及文件，目標為原儲存庫
`KicuasK/kAccounting-Extra-01` 的 `main`。前一基準提交為
`afba3de66661876e77c2e502f3b47dd71e2838ea`；新 Mac 應以遠端最新提交為準，
執行 `git log -1`、`git status` 確認取得版本並重新驗證。

本次只交付程式碼，不包含帳簿、備份、金鑰、Keychain、私人設定或臨時測試
節點登入狀態。GitHub 不代表既有財務資料已移轉。實際推送結果以 Git 遠端
確認為準；本文件不將未完成的正式安裝、私人代理或 UI 宣稱為完成。

## 接手先讀

1. `AGENTS.md`
2. 本文件（目前交接狀態）
3. `docs/VERSION01_DECISIONS.md`（既有核准政策，勿重複詢問）
4. `docs/DEVELOPMENT_PLAN.md`、`docs/API.md`、`docs/OPEN_QUESTIONS.md`
5. `docs/PRIVATE_INTEGRATION_2026-10-22.md`（最新實際驗收）
6. `docs/NON_UI_REMAINING_2026-10-22.md`、`docs/PRIVATE_ACCESS.md`

`docs/HANDOFF_NEW_MAC.md` 保留原始建置說明與歷史輪次；其中較舊的測試數、
未完成功能描述及 OS 版本不能覆蓋本文件與最新驗收。不能把所有歷史紀錄相加
當作本輪實測，也不能把暫時測試設定當作已完成正式安裝。

## 本輪更新（22:06–22:26）

接續紀錄見 [20 分鐘建置](BUILD_20MIN_2026-10-22.md)：修正取消關閉造成的
帳簿鎖遺留、關閉後請求接納、原生儲存路徑誤判；完成嚴格的停用啟動方案與
開發用 runtime 封裝。原生儲存、取消、復原資料夾選擇已用拋棄式資料實測。
這些結果不代表永久私人代理、簽署安裝或完整 UI 已完成。

## 現況與實際驗證

Version 01 尚未完成。前端仍是 Catalina 風格唯讀科目樹；完整記帳 UI 尚未建置。
本機非 UI 核心已完成並驗證：精確整數金額、原子平衡過帳、已過帳不可變、
稽核、帳簿明確選擇、附件串流、加密備份／新目的地還原、遷移前備份、
完整加密容器匯出／下載／上傳／匯入與中斷復原。

最後完整驗證：**651 個 Rust 測試通過、0 失敗、9 個刻意忽略；112 個 Python
工具測試通過；41 個前端測試通過**。格式、Clippy、文件、型別與前端正式建置
均通過，後端 Release 建置通過。忽略的容量／程序中斷測試曾有先前專項結果，
不宣稱這次常規驗證執行了它們。下一台電腦必須重新實測。

實際拋棄式資料流程分別通過本機 HTTP 與私人 HTTPS：選簿、附件串流／雜湊、
超過 JavaScript 安全整數範圍的金額、平衡過帳、加密備份、完整加密移轉、
匯入不切簿、明確切換與關簿。本機直接偽造完整代理標頭的請求仍回傳 403。
驗收工具先核對唯一 fixture token，再允許任何寫入。

## 私人連線的重要發現

- 官方 GUI 版 Tailscale 1.104.2 已登入且正常連線，但 system extension 沙盒
  拒絕兩個實測位置的 Unix 通道，包括 `/Library/Tailscale` 下專用目錄。
  不要重複嘗試相同路徑、降低 root peer 驗證或改信任未驗證的 TCP／標頭。
- 真實 HTTPS 正向驗收使用從官方固定 `v1.104.2` 原始碼建置的命令列 daemon，
  由使用者以管理員授權啟動獨立臨時節點。使用 userspace networking、記憶體
  ephemeral state、不接受 DNS／路由設定，不取代原 GUI，不安裝永久服務。
- Unix Serve 設定另需管理員授權。Tailscale 登入身分可為 `alice@github`，
  不能只接受電子郵件網域；仍逐字比對完整身分，不猜測或轉換大小寫。
- 使用者已在 HTTPS 授權頁取消勾選 Funnel，僅啟用 HTTPS。未啟用公開服務。
- 驗收後使用者停止所有臨時節點／代理；已確認程序與控制 socket 消失。
  原 GUI 仍在線、Serve 設定為空。這不表示正式相容部署已完成。
- 詳見 `docs/TAILSCALE_VARIANT_ACCEPTANCE.md`。暫存 binary、登入狀態、
  本機管理員啟動器及 private config 不應複製進原始碼交付。

## 剩餘非 UI 工作與順序

1. 將已驗證的相容命令列 transport 整理為正式安裝、啟動、停止、復原與更新
   流程；保留選簿、身分與來源驗證。正式使用不應要求多次手動開 Terminal。
2. 驗證正式封裝／簽章、登入啟動與 production Keychain 存取資格。已通過的
   專用新項目、隔離 Keychain 鎖定／解鎖和暫時 launchd 測試不能取代這些資格。
3. 補齊剩餘原生套件／備份／帳簿選檔與產品操作流程。接收請求儲存、新帳簿目的地、取消及復原資料夾已實測通過。`NativeHost`、正式啟動接線、世代保護、
   取消／回收與關閉競態測試已完成；不是實際產品按鈕或完整 GUI 驗收。
4. 完成後再跑完整驗證及 Release，更新交接結果，然後建置完整 UI。

先前對剩餘非 UI 工作估計為 **6–12 小時有效工時／約 1–2 工作天**，只是估計，
需要使用者授權、登入、重開機或憑證的等候時間另計。不能宣稱已全部完成。
完整 Mac Safari 操作驗收放在 UI 後；不將尚未存在的 UI 驗收設為開始 UI 的條件。

## 固定政策與界線

- 每次啟動先明確選簿；匯入／還原成功後保留目前帳簿，切換須另行選擇。
- Safari 匯出先下載到「下載項目」，使用者自行搬到外接磁碟。
- 完全驗證成功後只清除內部加密匯入暫存；失敗／中斷或身分不明的暫存保留。
- 實體 APFS／exFAT 外接磁碟驗收已移至 Version 02，既有程式與測試不刪除。
- 不設計應用程式密碼，不啟用公開服務／Funnel，不接觸真實帳簿或既有財務金鑰。
- 不猜測未決科目分類。UI 依核准互動規格與 Catalina Aqua 方向，見
  `docs/UI_STYLE_GUIDE.md` 及核准決策；不要重問已核准政策。

## 新 Mac 環境與建置

本輪實際主機：Apple Silicon、macOS **27.0（26A5416b）**。
較早交接的 macOS 26.6.2 是歷史環境。「Catalina」是外觀方向，不是實測相容宣稱。
工具固定為 Rust/Cargo 1.99.1、Node 24.22.1、npm 11.19.1；另需 Xcode Command
Line Tools、Python 3、zsh、ripgrep。Go 是本次替代 Tailscale 原始碼驗收的準備工具，
不是正常編譯記帳程式的必要相依套件。

取得**包含本輪修改的經使用者授權交付版本**後，先確認 Git 狀態，保留既有修改：

```sh
cd ~/Desktop/kAccounting-Extra-01
git status --short
git branch --show-current
git rev-parse HEAD
rustup show
node --version
npm --version
xcode-select -p
cargo fetch --locked
npm --prefix frontend ci
./scripts/verify.sh
cargo build --release --locked
```

保留 `Cargo.lock` 與 npm lockfile；已鎖定的依賴含 yanked 版本，不要為了安裝方便
重新生成整份 lockfile。遇到平台／套件問題先記錄與診斷。測試前先準備工具 PATH。
首次一般建置不需要 Tailscale、登入、管理員測試服務、真實帳簿或財務金鑰。
需要實際 OS／網路驗收時，另依文件使用明確的拋棄式 fixture 並取得所需操作授權。

## 原始碼與財務資料不會一起搬移

GitHub 只交付原始碼、測試、文件、migrations 和 lockfile。不能提交金鑰、資料庫、
附件、備份、移轉套件、`target`、`node_modules`、`dist`、本機私人設定與啟動器。
本機專用 Keychain 測試項目依使用者要求保留供檢查，不複製或刪除。
新 Mac clone／編譯成功不代表既有帳簿或 Keychain 已移轉；不得用真實帳簿測試。

待使用者日後明確要求上傳時，先稽核提交清單與敏感資料，再 commit／push，
記錄實際交付分支與 commit，並確認遠端確實包含文件與程式。現在尚未執行這一步。
