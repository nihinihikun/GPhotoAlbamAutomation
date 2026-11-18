# 構成

    SETUP.bat -> setup.ps1 -> gpae setup -> 専用ブラウザで通常ログイン
    RUN.bat -> gpae run -> ローカル WebSocket CDP -> Google フォトの読み取り RPC
                           -> 一覧取得 -> アルバム選択画面
                           -> Rust でページ検証 -> 詳細バッチと完了アルバムのチェックポイント
                           -> アルバム別 JSONL、一括 JSON、レポートの確定

- src/main.rs は CLI、設定、ブラウザセッション、結果の終了コードを管理します。0 は SUCCESS、2 は PARTIAL、1 は FAILED、3 は CANCELLED です。中止時のレポートには保存結果に応じて PARTIAL または FAILED と中止理由を記録します。
- src/config.rs は JSON とパスの検証を行います。src/browser.rs は専用 Chrome / Edge の管理、ローカルの CDP 接続、タブの移動と画像・動画リクエストのブロックを行います。
- src/photos.js は同じ Google フォトのページ内でのみ実行され、アカウント照合、許可 RPC の送信、レスポンスから必要な項目だけの射影を行います。CDP Runtime.evaluate で生成した関数の RemoteObject を Runtime.callFunctionOn で呼び出します。
- src/photos.rs は正規化したレスポンスの型検証と上限付き再試行を行います。HTTP 429、408、5xx と一時ネットワーク障害に対し最大3回試行します。認証・形式異常は停止します。
- src/export.rs は一覧と所属の継続トークン、ID 重複、申告数、完了後の一覧再照合を検証します。メディア詳細は40件までのバッチです。メモリには一覧のアルバムと、処理中のアルバムの ID 集合およびページを保持します。書き出し本体はディスクから逐次構築します。
- src/selection.rs は一覧取得後に同梱の Windows 選択画面を開きます。画面には名前と申告件数を渡し、戻された番号を元の一覧と照合します。

## 中断と再開

RUN は専用ブラウザを約30分ごとに切り替えます。区間の切り替え時は、同じアカウントで一覧を再取得し、取得開始時の一覧と照合します。完了したアルバムはそのまま使い、処理途中のアルバムは新しい接続で所属を確認しながら保存済みの詳細バッチを再利用します。

同じアカウントと出力先で実行し直すと、完了アルバムのチェックポイントを検査します。ヘッダー、順序、件数と SHA-256 を照合し、一覧のアルバム属性、現在の所属 ID・順序・ページから得た日時等のダイジェストを再取得して比較します。差があればアルバムを取り直します。途中のアルバムは所属一覧を先頭から読み、同じ順序のメディア ID で構成される最大40件の詳細バッチを再利用します。再利用する詳細情報は各照会時点の値です。中断時に確定した部分 JSON とレポートは別のファイル名で残ります。

一覧の再照合を終え、アクセスできないアルバムや申告件数より少なかったアルバムがなければ、その回で使ったチェックポイントを片付けます。途中失敗や件数の不足があったときは、再開用データを残します。以前の正常なバックアップと、今回のファイル名は別です。確定レポートは最後に出力します。

CDP は [Target](https://chromedevtools.github.io/devtools-protocol/tot/Target/) · [Runtime](https://chromedevtools.github.io/devtools-protocol/tot/Runtime/) · [Fetch](https://chromedevtools.github.io/devtools-protocol/tot/Fetch/) · [Page](https://chromedevtools.github.io/devtools-protocol/tot/Page/) を使用します。
