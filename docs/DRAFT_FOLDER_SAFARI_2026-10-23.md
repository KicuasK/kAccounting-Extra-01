# 草稿附件與資料夾移轉包 Safari 驗收

本輪只補驗既有流程。正式簽章持續暫緩，不接觸真實財務帳簿、備份或 Keychain。
使用一次性 fixture、記憶體假金鑰、真正 Safari 與原生檔案／資料夾選擇面板。

## 草稿附件 Safari 實際驗收

使用 fixture `79ccd2d33ff494f711af2e5cf5f62a10`，將空白行草稿儲存為分錄 2，
說明「草稿附件驗收」，初始版本 1、附件 0 份。測試檔為
`/private/tmp/ka-draft-safari-m0sltgk7/draft-attachment.txt`，70 bytes，初始 SHA-256
`e5a8b7c0700421f7a29a764255b6fe7deaa61320df8e9e048eff9d91789cf603`。

實際觀察如下：

1. 原生選檔面板取消後仍為 0 份附件、版本 1。
2. 選定檔案但未上傳時點儲存，顯示須先完成或取消附件選擇，沒有儲存分錄。
3. 新增附件確認畫面取消，再明確取消檔案選擇後，仍為 0 份附件、版本 1。
4. 重新選檔並另行確認新增後，清單為 1 份、70 bytes，草稿版本變為 2。
5. 驗證後顯示「完整性已驗證」，草稿版本維持 2。
6. 修改分錄說明但未儲存時，選檔、新增、移除均停用，未儲存文字保留；
   已用畫面截圖核對停用狀態。
7. 明確儲存後版本變為 3，附件操作恢復；另行確認移除草稿附件參照後，
   附件 0 份、版本 4。

操作後重新核對來源檔仍為 70 bytes，SHA-256 與上述初始值一致。

草稿附件使用樂觀版本檢查：上傳附 `expected_revision`，移除使用相同版本欄位。
UI 寫入後重讀草稿與附件清單；未知結果不自動重送。移除的是草稿參照，不能將
附件畫面清空誤記為已刪除來源或已清除所有加密物件。

## 資料夾移轉包 Safari 實際驗收

使用 fixture `5890bd948e6ea1ec85fe20a02fe1fecf`。先取消資料夾選擇面板，
沒有開始作業；再經過三個原生選擇面板，選定資料夾套件、接收請求及新帳簿
目的地，勾選並明確確認後，作業 `e0b8c83857a165ab4dbf7e56086a1018` 成功，
顯示 1 份附件。原 control book 仍為目前選取，不會自動開啟匯入帳簿。

明確更新帳簿清單、選取 `Imported-Directory-Book` 並開啟後，試算表借貸各為
`9,007,199,254,740,993`。開啟已過帳分錄 1，附件
`invented-directory-receipt.txt` 為 71 bytes，完整性驗證通過。

fixture 起初把接收請求建立為 0644，原生邊界依權限規則拒絕；只把測試請求
調整為 0600，再重新選取後成功。已修正 fixture，以 create-new 方式直接建立
0600 並檢查權限；自測與 clippy 通過。正式程式的權限檢查沒有放寬。

服務停止時的 receipt 顯示 `all_checks_passed=true`、`fresh_device_key=true`、
`attachment_plaintext_verified=true` 及 `posted_entry_exact=true`。7 個追蹤來源
檔案的 device、inode、長度及 SHA-256 均不變。原始紀錄位於
`/private/tmp/kaccounting-draft-folder-safari.log`。

這是同一台 Mac、假金鑰及兩個邏輯端點的驗收，不能代替實體跨 Mac 驗收；
錯誤接收者及既有目的地拒絕亦未在本輪 Safari 逐項操作。

兩個測試服務均已停止，本輪測試視窗已關閉，3000 埠無 listener。

## 完整驗證

完整 `./scripts/verify.sh` 通過：684 Rust、141 工具、185 前端通過，9 項
例行忽略，0 失敗；TypeScript 與 production build 成功。原始紀錄為
`/private/tmp/kaccounting-draft-folder-verify-final.log`。定向測試不重複累加至總數。
`cargo build --release --locked` 亦成功，紀錄為
`/private/tmp/kaccounting-draft-folder-release.log`。

本輪只修正驗收 fixture 與文件，正式程式及 lockfile 沒有變更；保留已驗證的
candidate5 開發封裝，不重新封裝相同正式程式。來源提交與外置證據由
`deliverables/draft-folder-verification/DELIVERY.json` 記錄，避免文件自參照提交 hash。
