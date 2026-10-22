import './style.css'

document.querySelector<HTMLDivElement>('#app')!.innerHTML = `
  <div class="app-shell">
    <header class="toolbar">
      <button class="toolbar-button" type="button" aria-label="收合側邊欄">☰</button>
      <h1>kAccounting-Extra-01</h1>
      <span class="environment-status">本機開發模式</span>
    </header>

    <div class="workspace">
      <aside class="sidebar" aria-label="主要導覽">
        <nav>
          <button class="nav-item is-selected" type="button" aria-current="page">會計科目</button>
          <button class="nav-item" type="button">日記簿</button>
          <button class="nav-item" type="button">總分類帳</button>
          <button class="nav-item" type="button">試算表</button>
          <button class="nav-item" type="button">資產負債表</button>
          <button class="nav-item" type="button">收支報表</button>
        </nav>
        <p class="sidebar-note">目前僅建立介面骨架，尚未連接財務資料。</p>
      </aside>

      <main class="content">
        <section class="content-header">
          <div>
            <p class="eyebrow">科目管理</p>
            <h2>會計科目</h2>
          </div>
          <button class="primary-button" type="button" disabled>新增科目</button>
        </section>

        <section class="table-frame" aria-labelledby="accounts-caption">
          <table>
            <caption id="accounts-caption">會計科目清單</caption>
            <thead>
              <tr>
                <th scope="col">科目名稱</th>
                <th scope="col">會計要素</th>
                <th scope="col">節點類型</th>
                <th scope="col">狀態</th>
                <th scope="col" class="amount-column">餘額</th>
              </tr>
            </thead>
            <tbody>
              <tr class="empty-row">
                <td colspan="5">
                  <strong>尚未建立科目資料</strong>
                  <span>下一階段會先審查資料模型與科目階層，再開放新增功能。</span>
                </td>
              </tr>
            </tbody>
          </table>
        </section>
      </main>
    </div>
  </div>
`

