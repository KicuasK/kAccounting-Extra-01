# 單台 Mac 正式安裝準備 — 2026-10-23

本輪採單台 Mac 保存帳簿與執行服務，其他裝置使用私人 HTTPS 瀏覽器連線。已完成下列使用者層準備與 transport-only 審查；尚未實際安裝、註冊或啟動正式服務，也沒有登入新節點。沒有 UI 建置、簽章或公證，既有試用服務保持運行，不採用其假帳簿為正式帳簿。

## 已完成的主機準備

- 新建私有主機根目錄 `~/Library/Application Support/kAccounting-Extra-01-host`。
- `books` 是新的空白 mode0700 帳簿集合；沒有建立真實帳簿、複製財務資料或存取 Keychain。
- `runtime-candidate6` 為既有 candidate6 的獨立副本，重新驗證六個封裝檔案。manifest SHA-256 為 `209a0c3daa4f9914eda3ead82b195a65ae0c022a30012b6c3028b5e21c0c1f06`；沒有重建前端。
- `~/.kaccounting-extra-01` 是新的空白 mode0700 ingress parent。預定 Unix socket 為 `~/.kaccounting-extra-01/accounting.sock`；尚未建立 listener。
- 沒有安裝使用者 LaunchAgent、寫入 root 部署目錄、建立正式 transport state、載入系統工作或改動既有 Tailscale GUI。

外置準備紀錄：`/private/tmp/kaccounting-host-user-preparation.json`。正式安裝檔案與帳簿不放進原始碼儲存庫。

## 先安裝代理檔案，再登入取得實際身分

新增 `scripts/prepare-private-transport.py`，用精確 binaries、來源聲明及其獨立 hash 產生 transport-only bootstrap review。它不依賴尚未取得的正式 hostname／登入身分；不產生假的 runtime config，不執行代理、不登入或設定 Serve。這解開「先要實際節點身分才能審查，但尚未安裝獨立節點」的循環。

`install-private-transport.py` 可接受完整部署 review 或此 bootstrap review；既有 root no-replace、降權驗證來源、來源身分與 hash 重查仍適用。bootstrap 的 `deployment_ready`、`installed`、`started`、`qualified`、`login_performed`、`serve_configured` 與 `funnel_enabled` 均為 false。來源 provenance 是操作者聲明，不能當作可重現建置證明。

外置交付目錄位於本聊天的 `deliverables`：

- `private-transport-bootstrap-review3`，review SHA-256：`d548379c77da656102ed35a0efc14d81d19846ad36c5eecce2eead0ecb8150ba`。
- `private-operations-kit3`：固定來源與 hash 的操作工具包；`Install-Private-Transport.command` 先核對已保留的內容，再由操作者在 Terminal 審閱與授權。
- `private-transport-bootstrap-delivery3.json`：保存工具包、入口、guard、review hash 與唯讀 plan 結果；`sudo_executed=false`、`installed=false`。

第一個管理員步驟僅安裝新的專用代理檔案，固定目的地 `/Library/Application Support/kAccounting-Extra-01/private-transport`；不註冊工作、不啟動、不登入、不設定 Serve。不要把 install receipt 的 installed=true 解讀為已上線。

## 後續順序與操作者界線

1. 操作者審查並授權 install-only。若出現 sudo／macOS 授權提示，由本人完成；不取得或記錄密碼。
2. 核對安裝 receipt 與實際檔案，再另行審查註冊／啟動專用 root daemon。保留原 Tailscale GUI、既有 state 與試用服務；新 daemon 使用 userspace networking、新 state 和獨立 control socket。
3. 對該獨立 control socket 執行登入流程，由本人在官方頁面登入。明確拒收 DNS 與 advertised routes；不使用 auth key、不重用 GUI state。
4. 登入後唯讀取得實際 `Self.DNSName` 與所屬 user login，請操作者確認唯一允許身分，再建立 mode0600 的 `{hostname, login, socket}` runtime config。無須請使用者憑空填寫尚不存在的完整 ts.net hostname。
5. 重新產生完整、非 example runtime review，審查使用者 LaunchAgent。對獨立節點先執行 disposable nonce probe，證明 kernel UID0 Unix peer 與真實 HTTPS／identity envelope，才設定確切的私人 Serve route 與帳務服務。每次 CLI 都指定新 control socket；不使用 default GUI socket。
6. 驗證專用節點 Serve 狀態與無 Funnel、精確 Host／Origin／own-user、同使用者直接偽造 Unix 請求被拒絕。清理限精確新 route／新工作，保留帳簿、金鑰與不明中斷 state。

完整操作及版本工具見 [runtime operations](PRIVATE_RUNTIME_OPERATIONS_2026-10-23.md) 與 [transport versioning](PRIVATE_TRANSPORT_VERSIONING_2026-10-23.md)。本輪準備不是登入、連線、root 安裝或永久部署成功證據。

## 尚未完成的實機門檻

管理員安裝／工作註冊與啟停、新節點登入、實際唯一身分確認、私有 HTTPS 與遠端裝置連線、第二使用者／tagged-client 拒絕、專用測試 Keychain 項目及取消／不可用情況、登出／登入與重開機、真實更新／回復均未在此輪完成。不得用單元測試、停用 plan 或歷史 ephemeral 成功結果替代。

不啟用 Funnel、公開服務或無認證 TCP 替代入口。遠端瀏覽器不提供本機原生檔案面板路徑。Developer ID 簽章、公證不列本次交付門檻；目前試用服務保持獨立。

## 非 UI 驗證

本輪 `scripts/verify-backend.sh` 通過：684 個 Rust 測試通過、9 項刻意忽略，271 個工具測試通過。紀錄位於 `/private/tmp/kaccounting-singlemac-nonui-verification.log`。此結果不表示實機安裝或遠端連線已完成；沒有執行前端建置。
