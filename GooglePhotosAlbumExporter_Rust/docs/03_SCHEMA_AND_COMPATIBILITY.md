# JSON スキーマ v2

一括出力は albums_<開始Unix秒>_<乱数>.json と同名の .report.json です。時刻は UTC の Unix 値です。保存本体には schema_version、account、source、status、started_unix_seconds、finished_unix_seconds、validation、error、albums が入ります。選択した範囲の情報をこの一括 JSON にまとめて記録します。

アルバム別 JSONL は一括 JSON と同じ名前の実行フォルダに保存します。各行が JSON オブジェクトで、`record` が `Album` の行にアルバム情報、`Media` の行に写真・動画ごとの情報、`End` の行に件数と検証情報が入ります。レポートの `album_files` にアルバム ID、保存先からの相対パス、各ファイルの SHA-256 を記録します。Windows でファイル名に使えない文字は置き換えます。

各 albums 要素に album_id、title、url、is_shared、owner_actor_id、owned_by_account、reported_count、membership_count、membership_count_verified、collected_unix_seconds、items が入ります。url は取得ページに同じアカウントスロット・ID の通常のリンクがある場合に記録します。共有キーを含むリンクは保存しません。is_shared は API の共有フラグです。owner_actor_id は API が返した所有者 ID です。所有者 ID とログインメールとの対応を確認できないため、owned_by_account は null です。

各 items 要素に id、position、file_name、taken_unix_milliseconds、creation_unix_milliseconds、mime_type、size_bytes、dedup_key が入ります。position は API が返した所属ページを連結した1始まりの取得順です。Google フォト画面で選択した並べ替え順とは別です。現在の読み取りマッピングで MIME を確定できないため mime_type は null です。ファイル名や撮影日時の不足も null です。同名アルバムは ID ごとに保持し、同じメディアが複数アルバムにあれば各アルバムに記録します。

レポートには列挙・保存件数、取得漏れ、件数確認不能、再開で再利用したアルバム、データ JSON の SHA-256 が入ります。通常の一覧 RPC に総件数がないため、取得したページが整合してもアカウント全体の全件性を確定できなければ PARTIAL です。FAILED の場合はレポートのみが出ることがあります。保存先自体の書き込みが失敗した場合はコンソールを確認してください。

`validation.album_count_enumerated` は選択画面に表示した全件数で、`album_count_selected` は今回の出力対象数です。一部を選んだ場合の SUCCESS は、選択した範囲の完了を表します。

所属一覧が末尾まで取得でき、実際の件数が申告より少ない場合は、取得できた項目を保存し、membership_count_verified を false にします。validation.album_member_count_mismatches に、該当アルバムの ID、申告件数、取得件数を記録します。この場合の結果は PARTIAL です。

## Album Backup

公開実装の albums はアルバム名をキーとするオブジェクトで、項目はファイル名と撮影時刻の組です。同名アルバム、元 ID、所属順、所有者、メタデータ欠落を保持できません。インポートは既定で秒精度に正規化し、照合候補が複数ある場合に一意性を保証できません。空アルバムも、写真の照合結果がないと復元対象に入りません。この版では GPAE の JSON を確定ファイルとします。以前に保存した JSON と互換ファイルは、従来のスキーマのバックアップとして保持できます。

参照 [Album Backup の書き出し形式と読み込み処理](https://github.com/lokster/Google-Photos-Album-Backup/blob/master/google_photos_album_backup.user.js#L494-L747)
