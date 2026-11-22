$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Drawing
function ComparePanelFrame($round,$before,$after,$focus) {
 $a=[Drawing.Bitmap]::new("$PSScriptRoot/$round/$before.png")
 $b=[Drawing.Bitmap]::new("$PSScriptRoot/$round/$after.png")
 try {
  $n=0
  $changed=0
  for($x=575;$x -lt 855;$x+=2){
   for($y=390;$y -lt 705;$y+=2){
    $n++
    if($a.GetPixel($x,$y).ToArgb() -ne $b.GetPixel($x,$y).ToArgb()){$changed++}
   }
  }
  [pscustomobject]@{round=$round;before="$before.png";after="$after.png";focus=$focus;panel_pixels_sampled=$n;changed_pixels=$changed}
 }finally{$a.Dispose();$b.Dispose()}
}
$results=@(foreach($entry in 'menu','palette','hotkey'){
 ComparePanelFrame 'replacement' "$entry-esc-before" "$entry-esc-after-first" '查找文字输入框，点击后输入的探针文字和光标可见'
 ComparePanelFrame 'escape-direct' "$entry-direct-panel" "$entry-direct-after-first" '面板打开后的默认焦点，未切换焦点'
})
$results+=ComparePanelFrame 'escape-direct' 'editor-focus-before' 'editor-focus-after-first' '点击目标文档正文，面板仍显示'
$results|ConvertTo-Json|Set-Content "$PSScriptRoot/escape-frame-diffs.json"
