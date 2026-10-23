# 接手開發紀錄 — 2026-10-23（台北）

## 來源、工作目錄與使用者決策

原桌面目錄的 `main` 停在 `afba3de66661876e77c2e502f3b47dd71e2838ea`，
143 個受追蹤檔案為未提交刪除，index 無暫存變更。全部保留，沒有還原、重設或
覆寫。`git fetch origin` 確認遠端交接點為
`171693561ffe64ff682074417923e11d4167bd78`，從此建立獨立工作目錄：

`/Users/kicuashsu/.codex/worktrees/accounting-ui-handoff/kAccounting-Extra-01`

分支 `codex/accounting-ui-handoff`。本輪成果尚未提交／推送；編譯產物與測試資料
不放進 Git。依指定順序讀完九份接手文件，並閱讀非 UI、runtime 與安裝工具文件。

本次對話的新決策優先於原圖與舊稿：

- 「科目代碼，可以刪除」：移除顯示代碼，不新增代碼分配或移轉契約。
- 「先保留借／貸方向」：先明列借／貸，未自行選擇正負號慣例。
- 「尚無憑證，先保留開發封裝」：保留開發交付，正式 Developer ID 簽章暫緩。

信用卡 subtype、額度、預繳／未償、超額及歷史限額政策仍待決。沒有搬用舊專案
規則，沒有將可用額度當成會計餘額，亦沒有重問已核准分錄編輯政策。

## 已完成的 UI 增量

- 02 清晰現代結構：全寬緊湊表格、底部摘要，除五類科目色標外採黑白灰。
- 明確選簿、切簿、關簿與更新清單；單一候選也不自動開啟。選項仍是後端提供的
  direct-child ID，不接收檔案路徑、不將帳簿或財務資料存入瀏覽器持久儲存。
- 名稱／完整上層路徑搜尋、科目性質篩選、右上停用入口與重設篩選。
  搜尋／停用結果展開必要上層路徑，原順序保留，摘要顯示完整路徑、性質、類型、
  狀態與餘額。僅科目名稱中文字採 DFKai-SB fallback；其他文字保持系統字體。
- 截至日期餘額接既有已過帳試算表 API，整數字串轉 BigInt，從不經 Number。
  核對日期、科目身分／名稱／要素、借貸週轉與淨額、唯一性及全表借貸合計。
  已驗證完整試算表中無活動者顯示 0；分類由下層合計；核對失敗顯示 —。
- 切簿、關簿與 pagehide 清除舊資料並取消請求；失敗的選簿操作仍重新核對服務
  狀態。保留重試、鍵盤焦點、樹展開方向鍵與窄視窗側欄焦點保護。

本輪只完成選簿與科目瀏覽 UI。新增／修改科目、日記簿、分錄編輯、報表頁、附件、
備份／移轉與原生選檔按鈕仍待續作；沒有把 disabled 按鈕或 13 張圖當成已完成功能。

## 已完成的非 UI 增量

選簿新增 opaque `selection_id` 與可選 `X-kAccounting-Selection` 並行保護。
新前端一律帶入，後端於同一 lease 內核對，過期請求在寫入／Keychain 讀取前拒絕。
即使 A → B → A 也不會復活舊世代；既有未帶 header 的整合呼叫保持相容。
詳見 [ADR 0052](decisions/0052-browser-selection-generation.md) 與 [API](API.md)。

原生工具發布改為 exclusive rename，消除 staging hardlink，讓實際編譯的
StoragePicker 符合封裝器 single-link 契約。保留不覆寫、權限、來源隔離與部分
失敗保留的界線，加入原生編譯輸出與封裝器互通回歸測試。Swift 實際編譯成功，
本輪未開啟任何原生選檔面板。

新增唯讀簽章預檢。檢查 binary 雜湊穩定、codesign strict verification、identifier、
Team ID、hardened runtime；提供預期 Team ID 時另核對 Apple anchor 與 Developer ID
Application 憑證 requirement。單靠 metadata 文字不算信任證據。不簽章、不讀私鑰、
不安裝、不啟動，也不宣稱 notarization、Keychain、登入或代理已驗收。

實際 Release 的 ad-hoc 簽章驗證有效，但沒有 Developer ID／Team ID 與 hardened
runtime，正式資格未通過（預檢退出碼 1 是此狀態的預期結果）。使用者選擇保留開發包。

## 本機實測

實際環境為 Apple Silicon、macOS **26.6.2（25G83）**、Rust 1.99.1、Node 24.22.1、
npm 11.19.1、Python 3.9.6、Xcode Developer 工具。並非交接文件的 macOS 27.0。

`cargo fetch --locked` 與 `npm --prefix frontend ci` 成功。
`Cargo.lock`、`frontend/package-lock.json` 均與交接提交相同，沒有重生成。

| 驗證 | 結果 |
| --- | --- |
| 接手基準完整 verify | 651 Rust、112 Python、41 前端通過；9 Rust 刻意忽略 |
| 整合後完整 `scripts/verify.sh` | 通過格式、Clippy、panic shortcuts、Rust 測試／文件、型別與前端建置 |
| 整合後 Rust | **654 通過、0 失敗、9 忽略** |
| 最後簽章 requirement 測試增加後，全工具重驗 | **122 通過** |
| 最終前端測試 | **54 通過** |
| `cargo build --release --locked` | 通過 |
| 真正 Swift helper 編譯與 single-link 檢查 | 通過 |
| 開發封裝及 trusted digest 獨立驗證 | 通過，5 個成員 |
| 停用啟動方案 | 已產生；disabled=true、installed=false、started=false |

原有 9 個容量／中斷測試仍刻意忽略，未宣稱本次執行它們。未變更會計核心、
migrations、精確金額或原子過帳／稽核政策。

Safari 真實操作使用 `local_acceptance frontend/dist --chooser-ui`：全新拋棄式資料、
記憶體假金鑰，fixture token `60a854e45d8f72a3ba01ef8658e9422d`。確認啟動未選簿、
選單選取不自動開簿、明確開簿、大於 JS 安全範圍的精確借貸餘額
`9,007,199,254,740,993`、分類合計、完整路徑搜尋、停用／重設、科目摘要、
切換空簿後清除前簿資料、明確關簿。本輪是本機 Safari 科目頁驗收，非完整 UI
或私人 HTTPS 驗收。驗收伺服器已 Control-C 正常關閉，3000 埠無殘留監聽。

## 保留的開發交付

持久輸出父目錄：

`/Users/kicuashsu/.codex/visualizations/2026/10/22/01a1644d-21cf-7191-9321-1c50a73b82f5/deliverables/`

- `kAccounting-Extra-01-development/`：後端 Release、前端、原生 StoragePicker、
  manifest 與 complete marker。沒有帳簿、Keychain、私人設定或任何財務資料。
- `local.kAccounting-Extra-01.plist`：指向本包與空白 `books/` 的停用方案。
  不是正式安裝，不是私人代理設定，不應移入 LaunchAgents 當成驗收完成。
- `signature-preflight.json`：未具備 Developer ID 正式資格的實際預檢紀錄。

Manifest SHA-256：
`df1a1e967ccf5963f22553c4ddb0cfb93b019e0b88184556e795b860f4ea166f`。
manifest 的 source_revision 是宣告的交接基準，role 明確為
`declared-base-not-build-attestation`；本輪修改尚未提交，不能當作乾淨 commit 建置證明。

本機完整紀錄：`/private/tmp/kaccounting-baseline-verify.log`、
`/private/tmp/kaccounting-final-verify.log`、`/private/tmp/kaccounting-final-tools.log`、
`/private/tmp/kaccounting-release-final.log`。這些日誌為本機證據，不是跨機交付路徑。

## 下一輪仍需完成

1. 科目維護與已核准的分錄編輯／明確儲存／過帳／更正 UI，以及所有報表頁。
2. 附件、備份、加密移轉／復原與原生操作的完整 UI 接線和 Safari 驗收。
3. 正式相容私人代理安裝、停止／復原／更新。保留 root peer、精確身分與來源驗證，
   不以 GUI 沙盒失敗的 socket 或未驗證 TCP／標頭替代。
4. 未來取得簽章身分後，續做正式封裝／簽章及 production Keychain、登入／重開機驗收。
5. 信用卡未決政策及未核准科目分類只在需要時取得決策；不自動播種示意分類。

Version 01 尚未完成。實體 APFS/exFAT 與 iPad 維持 Version 02，既有程式／測試保留。
原桌面未提交刪除完全保留；本輪 worktree 修改尚未提交，沒有推送或啟用持久服務。
