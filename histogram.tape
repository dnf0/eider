Output docs/src/histogram.gif
Set FontSize 14
Set Width 850
Set Height 600
Set Padding 20
Set Theme "Dracula"

Hide
Type "cargo build -p zarrduck > /dev/null 2>&1"
Enter
Type "clear"
Enter
Show

Type "./target/debug/zarrduck plot analysis.duckdb --plot-type hist"
Sleep 500ms
Enter
Sleep 3s
