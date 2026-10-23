# Desktop 原始碼整合與登入啟動（2026-10-23）

## 原始碼位置與保存

主要專案位置為 `~/Desktop/kAccounting-Extra-01`。原始 Desktop checkout 的整批刪除狀態已用命名 Git stash 保存，名稱為 `Preserve Desktop checkout state before restoring complete project 2026-10-23`；沒有強制重設或丟棄。main 已 fast-forward 接上完整開發歷史，再複製開發工作樹的21個待提交原始碼檔案（含 migration0011與日期選擇器），快照及hash清單保留於工作區外。

工作樹的原檔未覆寫或刪除；後續請以 Desktop checkout 繼續工作，避免舊工作樹重複提交。整合時發現 reports.rs 的 helper 位於 test module 後，僅調整排列以通過既有 Clippy 規則。

通用工具與產品原始碼納入 Git；22個含本機絕對路徑的操作程式碼副本放在 gitignored `scripts/private-acceptance-local/source-archive-2026-10-23`，僅供封存，勿直接執行相對目錄已改變的副本。該處 SOURCE_LOCATIONS.json 記錄原始可用入口。本機身分設定、帳簿、Keychain、測試備份與 Tailscale state 留在工作區外，不上傳 GitHub。第三方套件與 build cache 依 lockfile 重現，不當成產品原始碼提交。

## 實機登入啟動設定

操作者完成代理開機設定後，輸出紀錄已核對：代理新 registered plist SHA-256 為 `0a29a926629b10f204e2d35195806e299ca1eb9b1ac3427d48ac10f960281bce`，startup-enable confirmed，前後工作均 loaded／registered。沒有實際重開機。

app 已依既有管理工具先停止，再更新同一個 candidate6 的 `--start-at-login`，最後重新啟動。新 app plist SHA-256 為 `870945e49d8e54bd3a02ee045fb9a247a8cc0702bfcabd0276c8516877edd69d`；RunAtLoad=true。沒有部署本輪新增原始碼／migration，也未更換正式 runtime。私人 HTTPS 再次確認未選簿，localhost:3000 試用 PID82115 全程保留。

停止時實際發現舊正式 Unix socket 未自行清除，原有部署 revalidation 正確拒絕。核對固定工作 absent、lsof無持有者、連線 ECONNREFUSED、目錄0700／socket0600且為本人所有、inode與ctime未變後，將該socket改名保存，沒有刪除。紀錄留於外置 stopped-runtime-socket-recovery.json。之後原review重新核對成功，完成設定與啟動。

## 尚未合格的部分

登入啟動「設定完成」不等於「登入／重開機驗收通過」。現有candidate6會留下socket，未選簿的這次停止已實測出來；再次登入／重開機前仍需補齊安全的自動殘留socket復原／清理機制，否則新程序會依安全規則拒絕既有socket。不能以本次人工保留改名當成自動復原通過。

仍待：系統提示取消、第二使用者／tagged-client拒絕、維護時段真正登出／登入與重開機、真實不同版本的更新／回復。登出或重開機會中斷試用程序，因此未自行執行。migration0011啟用後的帳簿不能直接交給candidate6當作相容回退；舊runtime不部署新schema資料。

簽章、公證與UI建置依使用者指示不執行。

## 整合驗證

完整 verify-backend.sh 通過：688個 Rust 測試通過、9個刻意忽略；283個工具測試通過，格式、Clippy、文件檢查均通過。前端206個原始碼測試與TypeScript型別檢查通過。沒有執行UI build，lockfile未重新生成。結果紀錄位於工作區外 /private/tmp/kaccounting-desktop-consolidation-backend-fixed.log、kaccounting-desktop-source-tests.log、kaccounting-desktop-source-typecheck.log。
