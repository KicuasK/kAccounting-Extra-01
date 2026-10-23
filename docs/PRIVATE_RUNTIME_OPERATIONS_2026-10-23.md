# 單主機安裝、啟停、更新與復原工具

本輪只開發並以隔離檔案／模擬 launchctl 驗證工具，未安裝或啟停 macOS 工作、未登入或設定 Tailscale、未碰 Keychain；試用服務保持運行。沒有 UI 修改／前端建置或簽章公證。以下是操作介面，正式套用前仍需實際非 example 設定與獨立審查、操作者授權。

## 使用者帳務服務

`scripts/manage-private-runtime.py` 僅管理固定 `gui/<目前UID>/local.kAccounting-Extra-01` 與 `~/Library/LaunchAgents/local.kAccounting-Extra-01.plist`，不管理或終止其他服務。`Library/LaunchAgents` 須已存在且安全，不會自動建立或採用不安全目錄。

- `--action install`：必須提供新的非 example 審查包及外置 review hash；預設唯讀。`--apply` 以排他方式安裝 plist，但不 bootstrap，預設 `RunAtLoad=false`。install/update/rollback 可明確選 `--start-at-login` 產生 `RunAtLoad=true`；只是可選設定，不宣稱已完成登入／重開機驗收。
- `--action start`：須相同 review、review hash 與獨立保留的 installed plist hash；明確 `--apply` 後 bootstrap／kickstart 該工作。只表示要求載入與核對工作狀態，非遠端可用或 Keychain 合格證明。
- `--action stop`：核對 installed hash、已載入工作完整 path/program/argv 後，`--apply` 才 bootout 精確服務；不刪通訊檔、帳簿或金鑰。
- `--action status`：唯讀 launchctl print 並列目前 plist hash、待核對意圖。只有已觀察的明確「找不到此服務」結果才視為 absent，其他錯誤不猜測。不同 macOS 載入參數格式會拒絕變更，需檢查格式。
- `--action update`：工作必須已停止；新 runtime 封裝在獨立新目錄，新 review 綁定其內容。核對既有 installed hash，保留舊 plist 與操作意圖後切換引用，不覆寫 runtime。
- `--action rollback`：同樣只在停止後作用，須新提供指向舊 runtime 的有效 review，且 proposed plist 與工具先前保留的舊 plist 完全相同。這是程式版本回復，不回復資料庫 schema、帳簿或交易；新版若改資料格式需另外的相容性審查，不可當成資料復原。
- `--action reconcile --expected-state loaded|absent`：僅在使用者明確指定實際預期狀態、installed hash 與精確工作核對一致後，封存該工作的待核對意圖；不重送啟停、不繼續中斷更新，也不刪 pending 檔案。

除唯讀 status 外，預設皆為 plan，變更要另加 `--apply`；root 使用者不可套用這個使用者工具。install/update/rollback/start 要加 `--review ABS --expected-review-sha256 HASH`；既有工作變更要加 `--expected-installed-sha256 HASH`。hash 必須從獨立保留的紀錄核對，不把臨時讀取值直接視為授權。launchctl timeout 或無法核對結果會留下意圖鎖住後續變更，請先 status，再明確 reconcile。所有舊 runtime、備份 plist、帳簿、金鑰、代理 state 及不明 pending 檔均保留。

## 管理員專用代理檔案安裝

`scripts/install-private-transport.py --review ABS --expected-review-sha256 HASH --guard-sha256 GUARD_HASH --plan` 預設只重新核對來源與列出固定目的地。

管理員套用必須另以 Python `-I -S`、原登入使用者透過 sudo 的明確身分，以及 `--apply` 執行。本輪沒有執行。工具固定建立全新的 `/Library/Application Support/kAccounting-Extra-01/private-transport`；拒絕既存目錄或檔案，不採用原 GUI state。產生 root:wheel 私有 bin/state、固定雜湊的兩支代理與 guard、0600 完成 receipt 及停用的 daemon 審查 plist。未完成目錄保留，拒絕自動刪除或重用。

root 入口只載入標準函式庫；使用者的原審查驗證在有時間／大小上限且永久降權的子程序執行，root 再獨立核對外置 digest、審查內容與來源檔案身分／雜湊。guard 是明確 hash 固定的資料複製，不以 root 身分從 checkout import。這是新檔案安裝，不提供覆寫既有 transport 的升級或清除命令。

停用 plist 留在 ROOT 供管理工具審查；本輪沒有實際寫入 `/Library/LaunchDaemons`、bootstrap、Tailscale login 或 Serve。安裝 receipt 的 `installed=true` 僅指專用檔案出版，`launchdaemon_installed=false`、`started=false`、`qualified=false`、`activation_implemented=false`。

## 固定開機 guard

`scripts/private-transport-guard.py` 安裝至 root-owned `bin` 後，預定由固定 `/usr/bin/python3 -I -S` 載入。只接受固定主機路徑與固定代理參數，核對 root:wheel0700 ROOT/bin/state、單連結 binaries、receipt 與 guard SHA-256；不接受外部 argv、任意 executable、shell 字串或其他模組。

control parent 固定 `/private/var/run/kaccounting-private-transport`，缺少時建立，既存時必須 root:wheel0700 且完全空白；連 stale socket 或其他未知檔案都保留並拒絕，不自動 unlink。採用 macOS 精確的 `/private/var/run` root:daemon（uid0/gid1）0775 系統信任邊界；其他 group/world writable 父層拒絕，非可寫父層容許標準 root:admin。產品目錄不放寬。這項只通過隔離測試及系統父層唯讀檢查，沒有實際 root guard／重開機驗收。

## 剩餘正式操作門檻

正式 hostname／唯一登入身分、專用節點 Tailscale 登入與私人 Serve、工具的真實安裝／啟停與回復、登入自動啟動實機驗收、真實 Keychain、登出／登入與重開機仍未完成；不以工具測試或停用方案宣稱上線。任何後續啟用需先完成具體方案並請操作者審查／授權。繼續禁止 Funnel、公開服務或 TCP 身分驗證替代方案。

遠端瀏覽器的主機原生面板界線仍依前輪紀錄；還原與移轉保留主機端操作。Developer ID 與公證已排除，不列待完成門檻。

## 專用代理工作註冊與控制

新增 `scripts/manage-private-transport.py`，standalone 標準函式庫工具，以固定 root-owned install receipt 的獨立 SHA-256、固定 binaries／guard 雜湊及完整路徑核對。操作介面為 `register|start|stop|status|reconcile --expected-install-receipt-sha256 HASH`，預設唯讀 plan。實際套用限 macOS root 的 isolated Python 與明確 `--apply`，本輪未執行。

register 只向固定 `/Library/LaunchDaemons/local.kAccounting-Extra-01.private-transport.plist` 獨佔寫入完全固定的停用方案；拒絕既存名稱及碰撞工作。start/stop 只操作精確 `system/local.kAccounting-Extra-01.private-transport`，核對已載入 path、program 與完整 argv。start 包含精確服務的 launchctl enable（會留下持久的 override）與 bootstrap/kickstart；工具不暗中回復此 override，也不改其他工作。手動啟動不代表健康、登入或重開機合格。

管理紀錄在 ROOT 保留獨立的意圖與結果，未知狀態鎖住後續變更，須明確 reconcile；不刪 transport state、control socket 或原始 GUI 的任何資料。不提供覆寫既有 transport 的升級；帳務程式版本更新由前述使用者工具處理。實際私人路由與登入仍需另外的操作與驗收。

## Verification and retained operator kit

New tooling tests: 26 user lifecycle, 14 transport installation, 16 root control and 17 guard (73 new tests); with 148 existing tests, the complete tool suite passes 221 tests. These use disposable files and mocked privilege/launchctl boundaries, not actual root, Keychain or login/reboot qualification.

The frozen-source verify-backend log, locked Release log and independent source binding are retained externally in `deliverables/private-operations-verification/DELIVERY.json`. Actual read-only app status reports absent and no unresolved intents. The separate localhost:3000 trial process remains running.

Reconcile requires `--expected-state loaded|absent`; mismatches refuse. Confirmed success is recorded as confirmed, while an acknowledged known incomplete result is recorded as acknowledged-abort with actual_state and persistent-enable uncertainty. A root-private flock serializes mutations; no automatic resume or override reset occurs.

The reviewable `deliverables/private-operations-kit1` contains tools and this runbook only, with per-file SHA-256 manifest and externally bound source commit. No books, keys, private configuration, login state or new frontend/runtime build is included. Keep the separately verified candidate6 runtime. Actual production inputs and operator authorization are still required before apply; example identities are never install inputs.

Final frozen-source `verify-backend.sh`: 684 Rust passed, 0 failed, 9 intentionally ignored; 221 tooling tests passed. Formatting, Clippy and Rust documentation checks passed. Locked backend Release build passed. No frontend build was run.

## 最新增量：私人代理版本管理

上述「不提供代理升級」描述保留為先前工具的歷史邊界。本輪另增 standalone `scripts/update-private-transport.py`，提供停止後的升級、受限回復與中斷核對；詳見 [私人代理版本升級與回復](PRIVATE_TRANSPORT_VERSIONING_2026-10-23.md)。新版 guard 以協定與執行使用標記保護回復條件；舊 guard 無協定時拒絕，需另行移轉。這些隔離測試不代表真實管理員安裝、升級、私人路由或登入／重開機已驗收。工具包改以 `private-operations-kit2` 及外置 `private-transport-version-verification/DELIVERY.json` 保留，本輪不改 candidate6 或試用服務。
