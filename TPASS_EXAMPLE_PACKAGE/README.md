# TPASS 分錄範例包：給接手開發的 Codex

更新：2026-10-22。以 KAccountingSuper schema 34 為基準。
這是**去識別化、可重現的開發範本**，不是正式帳簿匯出、不是可直接匯入正式帳簿的檔案。
金額與站點均為測試範例；35 不是經此文件查證的官方票價。

## 先讀與執行順序

1. 閱讀 `TPASS_DEVELOPER_GUIDE.md`，理解購票、搭乘證據與攤銷的分離。
2. `examples/01-account-setup.json`：建立隔離帳簿、測試科目、期初與啟用。
3. `examples/02-buy-pass.json`：建立 NT$1,200 月票購買分錄與 30 日期別。
4. `examples/03-tpass-transit-ride.json`：逐筆記錄月票搭乘，只增加憑證與購票連結。
5. `examples/04-amortization-expected.json`：核對系統自動產生的 80 與 1,120 攤銷。
6. `examples/05-easycard-transit-and-journal.json`：一般悠遊卡實際扣款分錄，與 TPASS 分開。
7. `examples/06-delete-tpass-ride.json`：刪除搭乘證據，驗證購票與攤銷保持不變。

JSON 是帶有 action、input、capture、expected 的**案例描述格式**，不是應用程式原生匯入格式。
呼叫 API 時只把 `input` 作為 POST JSON body，路徑為 `/api/<action>`。
`${...}` 是執行時以該隔離帳簿回應 ID 替換的 placeholder，不可照字面傳送。
`capture` 僅是範例執行器的取值說明，不能傳進後端。
HTTP 回應的 `data.id` 在專案測試 helper 中已解包，因此 helper 使用 `result.id`。

## 借貸範例一覽

| 情境 | 借方 | 貸方 | 金額 | 備註 |
| --- | --- | --- | ---: | --- |
| 期初 | Cash 10,000 + EasyCard 1,000 | Capital | 11,000 | 只供隔離案例初始化 |
| 月票購買 | A - Prepaid Expense - TPASS | A - Cash | 1,200 | 購票與期別一併建立 |
| TPASS 逐筆搭乘 | 無 | 無 | 無 | Reference Price = 35，屬統計，不是費用 |
| 九月攤銷 | E - Transportation - TPASS | A - Prepaid Expense - TPASS | 80 | 9/30 23:59:59，2/30 天 |
| 十月攤銷 | E - Transportation - TPASS | A - Prepaid Expense - TPASS | 1,120 | 10/28 23:59:59，到期剩餘額 |
| 一般悠遊卡台鐵搭乘 | E - Transportation - TR | A - EasyCard - Example | 64 | 真正的電子錢包扣款 |
| 刪除 TPASS 搭乘 | 無 | 無 | 無 | 不沖回或刪除購票、攤銷 |

所有日期使用 2024 年，讓目前執行時會補記完整到期攤銷。這些日期是測試刻意選定，不是使用者實際出行日期。
若換成未來日期，不能再期待 report 已經出現兩筆攤銷。

## 最重要的防重複規則

- 02 使用 `tpass_buy` 後，不要再手動 `journal_save` 同一筆購票。
- 03 只存 Transit Ride，不要為 Reference Price 35 再建立費用分錄。
- 04 僅描述預期結果，**不得把兩筆 expected Journal 送進 journal_save**。
- 05 是額外的 EasyCard 情境，不是 TPASS 月票扣款或 TPASS 攤銷。
- 06 應呼叫 `delete_tpass_ride`，不是刪 purchase Journal 或 amortization Journal。
- 既有 Daily Ride Summary 保留但 UI 不可新增／修改；本包不建立摘要，避免與逐筆重複計數。

## 可執行驗證器

本包附 `verify.mjs`，會借用專案現有隔離測試 helper。它不是獨立安裝版應用程式，需要原專案、已編譯執行檔與 Playwright runtime。
它**不會連線目前 8000 正式服務**，會建立自己的暫存 home 與隨機連接埠。

在專案根目錄先建置，再執行：

```sh
swift build --scratch-path .build-v1
node docs/TPASS_EXAMPLE_PACKAGE/verify.mjs
```

若範例包被解壓到別處，指定專案位置；執行時的 working directory 仍須為專案根目錄：

```sh
KACCOUNTING_REPO=/absolute/path/to/KAccountingSuper node /absolute/path/to/TPASS_EXAMPLE_PACKAGE/verify.mjs
```

若實際 binary 不在 `.build-v1/debug/KAccountingSuper`，可依 `scripts/lib/runtime.mjs` 設定 `KACCOUNTING_BINARY`。
驗證器會核對：期別日期、兩筆攤銷與時間、純搭乘不新增 Journal、EasyCard 真正扣款、刪除後財務 line 保持不變，以及無 browser page errors。

## 圖片

![測試帳簿中的搭乘顯示](assets/tpass/journal-transit-ride.png)

圖片為 UI 範例；日期與金額可能與 JSON 案例不同，機器驗收以 JSON 與 verify.mjs 為準。

## 提供給另一個 Codex 的建議訊息

請先讀 README.md 與 TPASS_DEVELOPER_GUIDE.md，再查看 examples/ 的六組案例。
這些是隔離測試資料，不要套用到正式帳簿。
用 verify.mjs 重現既有行為，開發時保持「購票有分錄、TPASS 搭乘無新費用、自動攤銷固定 23:59:59、刪除搭乘不動財務」等不變條件。
若規則與程式衝突，先指出差異，不要猜測使用者真實資料或直接修改正式帳簿。
