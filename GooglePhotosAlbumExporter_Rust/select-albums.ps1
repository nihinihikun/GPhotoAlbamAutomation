param(
    [Parameter(Mandatory)]
    [string]$InputFile,
    [Parameter(Mandatory)]
    [string]$OutputFile
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$utf8 = [Text.UTF8Encoding]::new($false, $true)
$json = [IO.File]::ReadAllText($InputFile, $utf8)
if (!$json.TrimStart().StartsWith('[')) {
    throw 'アルバム一覧を読み込めませんでした。'
}
$albums = ConvertFrom-Json -InputObject $json
if ($albums -isnot [array]) {
    throw 'アルバム一覧を読み込めませんでした。'
}

$titles = [string[]]::new($albums.Count)
$titleCounts = [Collections.Generic.Dictionary[string,int]]::new([StringComparer]::OrdinalIgnoreCase)
for ($index = 0; $index -lt $albums.Count; $index++) {
    $album = $albums[$index]
    if ($album -isnot [pscustomobject] -or !$album.PSObject.Properties['title']) {
        throw 'アルバム一覧の形式が正しくありません。'
    }
    $title = ([string]$album.title -replace '[\x00-\x1F\x7F]', ' ').Trim()
    if (!$title) { $title = '名前なしのアルバム' }
    $titles[$index] = $title
    if (!$titleCounts.ContainsKey($title)) { $titleCounts[$title] = 0 }
    $titleCounts[$title]++
}

$entries = [System.Windows.Forms.ListViewItem[]]::new($albums.Count)
$titleNumbers = [Collections.Generic.Dictionary[string,int]]::new([StringComparer]::OrdinalIgnoreCase)
for ($index = 0; $index -lt $albums.Count; $index++) {
    $title = $titles[$index]
    $label = $title
    if ($titleCounts[$title] -gt 1) {
        if (!$titleNumbers.ContainsKey($title)) { $titleNumbers[$title] = 0 }
        $titleNumbers[$title]++
        $label = '{0}（同名 {1}/{2}）' -f $title, $titleNumbers[$title], $titleCounts[$title]
    }
    $entry = [System.Windows.Forms.ListViewItem]::new($label)
    $entry.Tag = $index
    $entry.ToolTipText = $label
    $count = $albums[$index].reported_count
    $countLabel = if ($null -ne $count) { '{0:N0}' -f $count } else { '—' }
    $null = $entry.SubItems.Add($countLabel)
    $entries[$index] = $entry
}

[System.Windows.Forms.Application]::EnableVisualStyles()
$form = [System.Windows.Forms.Form]::new()
try {
    $form.Text = 'Google フォトのアルバムを選択'
    $form.StartPosition = 'CenterScreen'
    $form.AutoScaleMode = 'Dpi'
    $form.AutoScaleDimensions = [Drawing.SizeF]::new(96, 96)
    $form.ClientSize = [Drawing.Size]::new(700, 520)
    $form.MinimumSize = [Drawing.Size]::new(550, 380)
    $form.Font = [Drawing.Font]::new('Segoe UI', 10)
    $form.BackColor = [Drawing.Color]::White
    $form.MinimizeBox = $false
    $form.TopMost = $true
    $form.Add_Shown({
        $null = $form.Activate()
        $form.TopMost = $false
    })

    $layout = [System.Windows.Forms.TableLayoutPanel]::new()
    $layout.Dock = 'Fill'
    $layout.Padding = [System.Windows.Forms.Padding]::new(20)
    $layout.RowCount = 5
    $layout.ColumnCount = 1
    $null = $layout.ColumnStyles.Add([System.Windows.Forms.ColumnStyle]::new('Percent', 100))
    foreach ($height in @(39, 36, 37)) {
        $null = $layout.RowStyles.Add([System.Windows.Forms.RowStyle]::new('Absolute', $height))
    }
    $null = $layout.RowStyles.Add([System.Windows.Forms.RowStyle]::new('Percent', 100))
    $null = $layout.RowStyles.Add([System.Windows.Forms.RowStyle]::new('Absolute', 47))
    $form.Controls.Add($layout)

    $heading = [System.Windows.Forms.Label]::new()
    $heading.Text = 'エクスポートするアルバム'
    $heading.Font = [Drawing.Font]::new('Segoe UI', 16, [Drawing.FontStyle]::Bold)
    $heading.Dock = 'Fill'
    $layout.Controls.Add($heading, 0, 0)

    $modes = [System.Windows.Forms.FlowLayoutPanel]::new()
    $modes.Dock = 'Fill'
    $modes.WrapContents = $false
    $layout.Controls.Add($modes, 0, 1)

    $all = [System.Windows.Forms.RadioButton]::new()
    $all.Text = 'すべて'
    $all.AutoSize = $true
    $all.Checked = $true
    $all.Margin = [System.Windows.Forms.Padding]::new(0, 0, 20, 0)
    $modes.Controls.Add($all)
    $subset = [System.Windows.Forms.RadioButton]::new()
    $subset.Text = '選んだアルバムだけ'
    $subset.AutoSize = $true
    $subset.Margin = [System.Windows.Forms.Padding]::new(0)
    $modes.Controls.Add($subset)

    $searchRow = [System.Windows.Forms.TableLayoutPanel]::new()
    $searchRow.Dock = 'Fill'
    $searchRow.RowCount = 1
    $searchRow.ColumnCount = 2
    $null = $searchRow.ColumnStyles.Add([System.Windows.Forms.ColumnStyle]::new('AutoSize'))
    $null = $searchRow.ColumnStyles.Add([System.Windows.Forms.ColumnStyle]::new('Percent', 100))
    $layout.Controls.Add($searchRow, 0, 2)
    $searchLabel = [System.Windows.Forms.Label]::new()
    $searchLabel.Text = 'アルバム名で検索'
    $searchLabel.AutoSize = $true
    $searchLabel.Anchor = 'Left'
    $searchRow.Controls.Add($searchLabel, 0, 0)
    $search = [System.Windows.Forms.TextBox]::new()
    $search.AccessibleName = $searchLabel.Text
    $search.Anchor = 'Left, Right'
    $searchRow.Controls.Add($search, 1, 0)

    $list = [System.Windows.Forms.ListView]::new()
    $list.AccessibleName = 'アルバム一覧'
    $list.View = 'Details'
    $list.FullRowSelect = $true
    $list.MultiSelect = $false
    $list.ShowItemToolTips = $true
    $list.HeaderStyle = 'Nonclickable'
    $list.Dock = 'Fill'
    $titleColumn = $list.Columns.Add('アルバム名', 490)
    $countColumn = $list.Columns.Add('写真・動画', 115)
    $countColumn.TextAlign = 'Right'
    $list.Add_Resize({
        $titleColumn.Width = [Math]::Max(160, $list.ClientSize.Width - $countColumn.Width - [System.Windows.Forms.SystemInformation]::VerticalScrollBarWidth)
    })
    $layout.Controls.Add($list, 0, 3)

    $footer = [System.Windows.Forms.TableLayoutPanel]::new()
    $footer.Dock = 'Fill'
    $footer.RowCount = 1
    $footer.ColumnCount = 3
    $null = $footer.ColumnStyles.Add([System.Windows.Forms.ColumnStyle]::new('Percent', 100))
    for ($index = 0; $index -lt 2; $index++) {
        $null = $footer.ColumnStyles.Add([System.Windows.Forms.ColumnStyle]::new('AutoSize'))
    }
    $layout.Controls.Add($footer, 0, 4)
    $status = [System.Windows.Forms.Label]::new()
    $status.Dock = 'Fill'
    $status.TextAlign = 'MiddleLeft'
    $footer.Controls.Add($status, 0, 0)

    $cancel = [System.Windows.Forms.Button]::new()
    $cancel.Text = 'キャンセル'
    $cancel.DialogResult = 'Cancel'
    $cancel.AutoSize = $true
    $cancel.MinimumSize = [Drawing.Size]::new(100, 30)
    $cancel.Anchor = 'Right'
    $footer.Controls.Add($cancel, 1, 0)
    $form.CancelButton = $cancel

    $export = [System.Windows.Forms.Button]::new()
    $export.Text = 'エクスポート'
    $export.DialogResult = 'OK'
    $export.AutoSize = $true
    $export.MinimumSize = [Drawing.Size]::new(115, 30)
    $export.Anchor = 'Right'
    $footer.Controls.Add($export, 2, 0)
    $form.AcceptButton = $export

    $selection = [Collections.Generic.HashSet[int]]::new()
    $view = @{ refreshing = $false }

    function Update-SelectionSummary {
        if ($all.Checked) {
            $status.Text = '全 {0:N0} 件' -f $entries.Count
        } else {
            $status.Text = '{0:N0} / {1:N0} 件を選択' -f $selection.Count, $entries.Count
        }
        $export.Enabled = $all.Checked -or $selection.Count -gt 0
    }

    function Update-AlbumList {
        $view.refreshing = $true
        $list.BeginUpdate()
        try {
            $list.Items.Clear()
            $list.CheckBoxes = $subset.Checked
            $query = $search.Text.Trim()
            foreach ($entry in $entries) {
                if ($query -and $titles[$entry.Tag].IndexOf($query, [StringComparison]::CurrentCultureIgnoreCase) -lt 0) {
                    continue
                }
                $entry.Checked = $subset.Checked -and $selection.Contains([int]$entry.Tag)
                $null = $list.Items.Add($entry)
            }
        } finally {
            $list.EndUpdate()
            $view.refreshing = $false
        }
        Update-SelectionSummary
    }

    $list.Add_ItemCheck({
        param($sender, $eventArgs)
        if ($view.refreshing -or !$subset.Checked) { return }
        $index = [int]$sender.Items[$eventArgs.Index].Tag
        if ($eventArgs.NewValue -eq [System.Windows.Forms.CheckState]::Checked) {
            $null = $selection.Add($index)
        } else {
            $null = $selection.Remove($index)
        }
        Update-SelectionSummary
    })
    $search.Add_TextChanged({ Update-AlbumList })
    $subset.Add_CheckedChanged({ Update-AlbumList })
    Update-AlbumList

    $result = $form.ShowDialog()
    $response = 'null'
    if ($result -eq [System.Windows.Forms.DialogResult]::OK) {
        $indices = @()
        if (!$all.Checked) { $indices = @($selection | Sort-Object) }
        $response = ConvertTo-Json -Compress -InputObject @{
            all = $all.Checked
            indices = $indices
        }
    }
    [IO.File]::WriteAllText($OutputFile, $response, $utf8)
} finally {
    $form.Dispose()
}
