# 開發接續筆記

這份筆記用來記錄目前 `terminal-file-manager` 的互動規則、已完成功能與後續建議，讓之後繼續開發時可以快速接上。

## 目前已完成的重點

- 支援 `config.toml` 外部設定檔讀取。
- 支援主題切換與 theme 架構整理。
- 重新命名採用接近 mature-reference 的 inline 視窗。
- rename 已支援較完整的 Vim 操作：
  `Esc` 切到 normal mode、`i`/`a`/`A` 進入 insert mode、`w`/`b`/`e` 移動游標。
- 複製、剪下、貼上已支援批次操作。
- 同目錄複製時會自動處理重名。
- `a` 可建立檔案或資料夾。
- 若輸入像 `test/gg.txt`，會自動建立中間目錄與檔案。
- `f` 可開啟 filter，輸入時即時過濾。
- filter 的 `Esc` 流程是兩段式：
  第一次離開輸入，第二次離開 filter 畫面。
- `,` 可開啟排序面板，並以單鍵切換排序方式。
- 右側排序資訊會依排序模式顯示對應內容，例如時間、附檔名、大小。
- preview 已支援 focus mode，且可與列表區塊交換比例，讓預覽放大。
- 預覽中已支援搜尋。
- pane 已支援分割與關閉。
- preview 現在已改成 pane 各自獨立的狀態，不再是全域共享。

## 目前的 `V` 視覺標記規則

- 按下 `V` 會進入視覺標記模式。
- 在視覺標記模式中，用 `j/k`、`gg/G` 移動游標來選取一段範圍。
- 按下 `V` 或 `Esc`，都會結束這次範圍選取，並把這段加入已標記項目。
- 離開本次選取後，可以移動到別處，再按一次 `V` 繼續補選新的範圍。
- 如果目前不在視覺標記模式，但畫面上還保留已標記項目，按下 `Esc` 會一次清掉全部標記。
- 列表左側的標記欄位目前規則：
  沒有任何標記時不顯示額外縮排；只要有任一項被標記，欄位寬度就固定保留，避免列表跳動。

## 這次最後補上的行為

- 進入子目錄後再回上一層，游標會回到剛剛離開的那個目錄，不會跳回第一列。
- 在 preview 中可用 `Ctrl-w` 切換 pane。
- preview 狀態只屬於原本那個 pane；切到其他 pane 不會被強制進入 preview。

## 建議下一步優先做的功能

- global search
  使用者前面有提到未來想把 `s` 留給 global search，這會是很自然的下一步。
- 更完整的 visual mode
  目前 `V` 已可做範圍標記，但之後可以補更多 Vim 式移動與操作整合。
- 多 pane 專屬操作
  之後可以思考不同 pane 的獨立標記、獨立 clipboard 呈現、或 pane 間的操作提示。
- 操作提示面板優化
  目前 help 仍偏簡單，之後可以做更清楚的 mode 狀態提示。
- 設定檔擴充
  可以把主題、預覽比例、是否顯示 hidden、預設排序方式等都開放到 `config.toml`。

## 目前最近的 commit

- `de4873b` `Add visual selection workflow`
- `9d78ace` `Preserve selection when leaving directories`
- `2d9e0e4` `Add development notes`
- `8532293` `Scope preview mode to each pane`

## 接續開發時建議先做的事

1. 先執行 `cargo test`，確認基線狀態正常。
2. 打開這份筆記確認上次討論到哪裡。
3. 依照下一步優先順序，從 global search 或 visual mode 強化開始。
