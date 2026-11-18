# リリース作業

初回リリースは Google Photos Album Exporter v0.2.0 の Windows x64 版です。

## 公開前の確認

1. `GooglePhotosAlbumExporter_Rust/VERIFY.bat` を実行します。
2. リポジトリのルートで次のコマンドを実行し、Windows 用 ZIP と SHA-256 を `target/releases/` に作成します。パッケージ作成には Rust と MSVC のビルド環境を使います。
3. 配布用 ZIP を展開し、`SETUP.bat` と `RUN.bat` の起動を確認します。
4. 設定ファイル、ブラウザプロファイル、書き出し結果を含めずに配布します。サンプル設定は `config.example.json` を使います。

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package-windows.ps1
```

## GitHub での公開

1. このローカルリポジトリを公開先の GitHub リポジトリへ push します。
2. GitHub の Releases で `v0.2.0` を指定し、新しいリリースを作成します。
3. Windows 用の `GooglePhotosAlbumExporter-v0.2.0-windows-x64.zip` と、その `.sha256` を添付します。ソース配布 ZIP を併記する場合は `GooglePhotosAlbumExporter-v0.2.0-source.zip` と、その `.sha256` も添付します。
4. リリース本文に、アルバム選択、アルバム別 JSONL、途中再開が使えること、Windows exe 版は Rust 不要であることを記載します。

利用者には Windows 用 ZIP を展開し、`SETUP.bat` → `RUN.bat` の順に起動するよう案内します。
