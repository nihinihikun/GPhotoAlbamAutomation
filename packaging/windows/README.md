# Google Photos Album Exporter v0.2.0 Windows x64

64ビット版 Windows 用です。Windows 11 で動作確認しました。Rust や開発用ツールのインストールは不要です。Chrome または Edge と、Windows 標準の PowerShell を使います。

## 初回の設定と実行

1. ZIP を展開し、展開先の `SETUP.bat` を開きます。
2. メモ帳で設定が開いたら、`account` に取得元の Google メールアドレスを入力して保存します。`output_dir` の初期値 `exports` は、このアプリのフォルダ内に保存する指定です。別の保存先には `D:/GooglePhotosAlbumBackup` のように書けます。
3. メモ帳を閉じると専用ブラウザが開きます。対象アカウントで Google フォトにログインし、アカウントを確認して、ブラウザの「…」→「終了」で閉じます。Chrome / Edge が見つからない場合は Chrome のインストールを開始します。
4. `RUN.bat` を開き、アルバム選択画面で「すべて」または必要なアルバムを選んで「エクスポート」を押します。
5. 完了時に表示される保存先を開きます。実行ごとのフォルダに `アルバム名.jsonl`、同じファイル名が重なる場合は `アルバム名 (2).jsonl` のように保存します。一括 JSON と結果レポートも出力します。

実行中は `P` + Enter で一時停止、`R` + Enter で再開、`Q` + Enter で中止できます。途中保存からの再開は、同じ設定で `RUN.bat` をもう一度開き、同じアルバムを選びます。

保存対象はアルバム名、所属する写真・動画のファイル名、撮影日時などのメタデータです。取得件数の差や情報不足があると `PARTIAL` が表示されます。レポートで確認できます。ファイル形式は [出力仕様](docs/03_SCHEMA_AND_COMPATIBILITY.md) を参照してください。

## コマンドで使う場合

展開先のターミナルで次のように実行できます。

```powershell
.\gpae.exe check --config config.json
.\gpae.exe run --config config.json
.\gpae.exe run --all --config config.json
```

最後のコマンドは選択画面を開かずにすべて取得します。

## 片付ける場合

専用ブラウザとアプリを終了して `UNINSTALL.bat` を開きます。専用プロファイルや設定の退避先を確認して進めてください。書き出したデータや必要な退避データを保存してから、展開したアプリのフォルダを削除できます。

ライセンスは [MIT](LICENSE) です。参照元の Google Photos Toolkit の [ライセンス](docs/TOOLKIT_LICENSE.txt)、[依存ライブラリの表記](THIRD_PARTY_NOTICES.txt) を同梱しています。Google フォト Web の非公開 API を利用しており、Google 側の仕様変更によって処理が停止する場合があります。
