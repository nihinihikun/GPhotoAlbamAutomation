# Google Photos Album Exporter

Google フォトのアルバム名、写真・動画の所属、ファイル名、撮影日時などを JSON / JSONL に保存する Windows 用ツールです。

## Windows exe 版を使う

このリポジトリの **Releases** から `GooglePhotosAlbumExporter-v0.2.0-windows-x64.zip` をダウンロードし、ZIP を展開してください。Rust や開発ツールのインストールは不要です。Chrome または Edge と Windows 標準の PowerShell を使います。Windows 11 x64 で動作を確認しています。

1. 展開先の `SETUP.bat` を開きます。
2. メモ帳で開いた `config.json` の `account` に取得元の Google メールアドレスを入力します。`output_dir` で保存先を指定し、保存してメモ帳を閉じます。
3. 専用ブラウザで Google フォトにログインし、アカウントを確認します。ブラウザ右上の「…」→「終了」で閉じます。
4. `RUN.bat` を開きます。選択画面で「すべて」または必要なアルバムだけを選び、「エクスポート」を押します。
5. 完了時に表示されたフォルダとレポートを確認します。

保存先の初期値 `exports` は、展開したアプリのフォルダ内です。別の保存先には `D:/GooglePhotosAlbumBackup` のように `/` 区切りのパスを書けます。Chrome / Edge が見つからない場合は、SETUP が Chrome のインストールを開始します。

## 出力と操作

実行ごとのフォルダに `アルバム名.jsonl` を保存します。ファイル名が重なる場合は `アルバム名 (2).jsonl` のように番号を付けます。選択した範囲をまとめた `albums_*.json` と、取得件数や確認結果を記録した `albums_*.report.json` も出力します。

| 操作 | 実行画面での入力 |
| --- | --- |
| 一時停止 | `P` + Enter |
| 再開 | `R` + Enter |
| 途中結果を保存して中止 | `Q` + Enter |
| 停止後の再開 | 同じ設定で `RUN.bat` を開き、同じアルバムを選択 |

処理件数と経過時間が表示されます。長時間の取得は約30分ごとに専用ブラウザを切り替え、途中保存を引き継いで進めます。

Google フォトの申告件数と取得件数に差がある場合や、全件性を確認できない場合は `PARTIAL` と表示します。取得できた内容と確認できなかった項目はレポートに記録します。

## ソースから実行・検証する

ソースは `GooglePhotosAlbumExporter_Rust` にあります。ソース版の `SETUP.bat` は Rust と Windows のビルド環境も確認し、不足分の導入を開始します。詳しくは [ソース版の手順](GooglePhotosAlbumExporter_Rust/docs/01_SETUP_WINDOWS_JA.md)を参照してください。

```powershell
cd GooglePhotosAlbumExporter_Rust
.\SETUP.bat
.\RUN.bat
```

`VERIFY.bat` は書式、Rust の模擬テスト、Clippy、リリースビルド、JavaScript の模擬テストを実行します。検証には Node.js も必要です。

## 詳細

- [出力形式](GooglePhotosAlbumExporter_Rust/docs/03_SCHEMA_AND_COMPATIBILITY.md)
- [検証結果](GooglePhotosAlbumExporter_Rust/docs/TEST_RESULTS.md)
- [安全性とデータの扱い](GooglePhotosAlbumExporter_Rust/docs/02_SECURITY.md)
- [API 調査と参照元](GooglePhotosAlbumExporter_Rust/docs/API_RESEARCH.md)
- [リリース作業](docs/RELEASE_JA.md)

Google フォト Web の非公開 API を利用しており、Google 側の仕様変更で動作が変わる場合があります。

ライセンスは [MIT](LICENSE) です。参照元の Google Photos Toolkit の [ライセンス](GooglePhotosAlbumExporter_Rust/docs/TOOLKIT_LICENSE.txt)を同梱しています。
