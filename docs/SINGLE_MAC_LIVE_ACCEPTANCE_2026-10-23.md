# 單台 Mac 私人服務：實機驗收增量（2026-10-23，台北時間）

本輪保留 localhost:3000 的試用服務（PID 82115）。正式服務使用獨立的私人 Unix socket 與已驗證 candidate6；未重新建置 UI、未簽章公證、未動其他工作中的後端／前端修改。

## 已測得的結果

- 操作者已完成專用 root daemon 安裝、註冊、啟動及 Tailscale 登入。專用節點 Running／Online，原 Serve 設定為空；未沿用原 GUI state。
- 一次性 nonce HTTPS 測試通過，實際 Unix peer UID0、精確 hostname／HTTPS／使用者登入標頭吻合，無 Funnel；本次測試 socket 與路由清理成功。
- 操作者確認唯一登入身分後建立 private config、安裝並啟動正式 user LaunchAgent。私人 Serve443 僅代理此正式 socket；首頁、正常 TLS 與安全回應標頭驗證通過。
- 操作者回報另一台登入同一身分的裝置可看到帳簿選擇畫面。此項記為使用者實機確認，非本代理自行控制另一裝置的結果。
- 本代理再次 GET 私人 /api/session 與 /api/session/books：未選簿、空白集合、no-store。跨來源 Origin 及錯誤 Host 均403。一般使用者直接對正式 socket 偽造正確代理身分欄位亦403。
- 未選簿時探查 native 路徑得到409（session fallback 的 selection_required），不能以這個請求證明 native 路由不存在；原始期待404的失配證據保留，審查結果只列為未定，不宣稱通過該路由驗收。

## 已安裝版本的 Keychain／備份還原

在工作區外的全新空白測試帳簿執行既有 candidate6 binary，SHA-256 `1ccd6191c955fc8a64ac94c214f473e60a8137080cf6cf6d03047526ded8485b`，與封裝 manifest 一致。

新測試金鑰建立、另一個程序重新讀取並產生已驗證加密備份、還原到全新目錄均成功。還原帳簿尚無 managed-backup identity 時，備份正確拒絕；明確執行 setup-backup-identity 後再次備份成功。沒有使用正式帳簿；測試帳簿、備份與新 Keychain 項目保留供檢查。未測提示取消、登入後 ACL 或重開機。

實機紀錄保存在聊天外置 deliverables 的 private-probe-evidence3-*、private-accounting-connection-*、private-live-security-*、installed-keychain-acceptance-*；不得將其金鑰、帳簿、私人設定或使用者身分提交到原始碼。

## 開機啟動工具增量（尚未套用）

manage-private-transport.py 增加 startup-enable／startup-disable／startup-reconcile。必須同時提供獨立保留的 install receipt SHA-256 與 --expected-registered-plist-sha256；預設 plan。只接受原停用 schema 或精確啟動 schema（Disabled=false、RunAtLoad=true、KeepAlive=false）。保存原 plist、持久 intent，排他寫入新檔後原子替換、fsync 與重核；異常保留 pending，後續修改拒絕直到專屬 reconcile。共用原操作鎖與版本 pending gate。

這些命令不執行 launchctl enable／disable／bootstrap／bootout／kickstart，也不重載運行中的工作；不變更目前服務。startup-enabled schema 的後續 status/start/stop/reconcile 須明確固定當前 registered plist hash。回報保持 reboot_start_qualified=false，不能以檔案設定或隔離測試冒稱開機通過。

## 剩餘門檻

1. 審查並套用專用代理開機啟動及 app 登入啟動。app 現有 RunAtLoad=false；使用者工具支援明確 --start-at-login，但更新必須依停止／更新流程，不直接改 live plist。
2. 提示取消／互動不可用的實機錯誤，以及正式版本經登出／登入、重開機後的 Keychain 存取、control parent 安全重建、節點身分保留、chooser 未選簿及私人 HTTPS。此類操作會中斷試用服務，必須先由操作者安排維護時段。
3. 真實另一使用者／tagged-client 拒絕測試；目前錯誤 Host／Origin 與偽造 Unix 請求不能替代它。
4. 真實版本更新／回復需維護時段及真正不同、已審查的新版本。已執行或 state 變更的代理版本不可使用未使用新版的快速回復。保留舊 binaries、state 及意圖紀錄。

正式上線整體 qualification 仍未全部完成；簽章公證按使用者要求不列門檻。

## 本輪驗證

凍結後完整工具套件 283 tests passed（/private/tmp/kaccounting-live-nonui-tools-startup.log）；新增前基準為271。獨立審查另確認 manager30／guard21／updater29 全數通過，這些包含在283內，不重複累加。git diff --check 通過。此輪僅修改2個scripts與本文件，未重建UI或backend；其他工作正在修改後端與前端，因此未將那些未凍結變更納入本輪提交或宣稱已完整驗證。
