# 單台 Mac 主機：非 UI 增量（2026-10-23）

使用者範圍：帳簿與後端只在此 Mac，其他裝置以私人遠端瀏覽器連線。Developer ID 簽章與公證不做；實體跨 Mac 移轉不列日常上線門檻，保留既有功能。此輪沒有修改前端、執行 npm 或重新建置 UI，candidate6 資產維持不變。正在運行的假資料試用服務亦保留。

## 已完成

- 新增 `prepare-private-deployment.py --verify … --expected-review-sha256 … --revalidate-inputs` 唯讀部署輸入檢查。審查輸出新增版本化 input snapshot，綁定 runtime 整樹身分與 private config 路徑。重核封裝、代理程式、來源聲明、設定、帳簿集合與停用啟動方案，拒絕同內容但不同 inode 的替換；不執行任何來源 binary。
- 舊版離線驗證保留；缺 snapshot 或 example-only 的資料不可當成正式輸入通過。即使重新檢查成功，installed/started/qualified/deployment_ready/atomic_installation_boundary 仍為 false，不代表已安裝或可直接啟動常駐服務。
- 用 candidate6 與既有獨立固定雜湊的代理檔案建立新的停用 example 審查包，離線驗證成功；新檢查正確拒絕將 example 用於正式部署。不推測實際主機名或登入身分。
- 重核 candidate6 外置 manifest SHA-256：`209a0c3daa4f9914eda3ead82b195a65ae0c022a30012b6c3028b5e21c0c1f06`。沒有重新封裝；locked 後端 Release 建置通過。

## 驗證

後端完整驗證：684 Rust passed、0 failed、9 ignored；格式、Clippy、Rust 文件檢查通過。基準工具套件 141 tests；新增工具檢查後完整工具套件為 148 tests passed、0 failed（11.746 秒）。19 deployment tests、24 launcher tests 定向通過，不重複累加。

另行以 Release 執行 5 個專項，每項 1 passed、0 failed：

| 專項 | 測試本體耗時 |
| --- | ---: |
| 64 MiB 真實子程序 SIGKILL 後移轉復原與來源保留 | 2.25 秒 |
| 128 MiB 真實子程序 SIGKILL 後私有暫存復原 | 1.12 秒 |
| 250 MiB 附件加密串流及完整性 | 2.03 秒 |
| 250 MiB 完整接收者移轉包往返 | 13.06 秒 |
| 250 MiB HTTP 附件串流 | 3.14 秒 |

只使用各測試自己建立的假資料／記憶體金鑰與子程序，不殺產品或使用者程序。上述 5 項是常規 ignored 中的 5 個主測試；2 個 child helper 不獨立執行，真實 Keychain 與實體媒體測試未執行。不能把專項加到684常規結果並宣稱無忽略。

## 遠端操作界線與剩餘工作

正式認證代理成立後，遠端可使用既有帳簿選擇、帳務、一般附件串流與主機既定 backups 位置的手動備份。私人入口排除原生面板路由；主機原生選檔、新帳簿目的地、還原及移轉仍需要主機端真正的選檔 capability。這不是完整的純遠端管理介面，亦不能假設 loopback/private 的分開服務模式共用選簿工作階段。

仍待實作／審查：受保護且不覆寫既有項目的正式安裝、啟停／更新／復原與開機 control parent 重建工具。仍待操作者：Tailscale 實際節點登入、正式身分設定、管理員安裝授權、專用真實 Keychain 與登出／登入／重開機驗收。例子、假資料與離線檢查不可替代這些結果。

原始 log、測試 summary、封裝驗證與 example 審查位置由工作區外 `deliverables/single-host-nonui-verification/DELIVERY.json` 記錄。不提交帳簿、密鑰、runtime 設定或生成物。
