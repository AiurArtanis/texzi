function K($label,$keys){Invoke-TexziWindow @{action='keys';label=$label;keys=$keys}}
function P($label,$x,$y){Invoke-TexziWindow @{action='click';label=$label;x=25+$x;y=25+$y}}
function T($label,$s){Invoke-TexziWindow @{action='text';label=$label;text=$s}}
function EnterReplace($entry,$tag){
 switch($entry){
  'menu' {
   K "$tag-menu-open" @(18,69)
   Invoke-TexziWindow @{action='title-menu';label="$tag-menu-popup"}
   $p=Get-Content "$run/$tag-menu-popup.json"|ConvertFrom-Json
   Invoke-TexziWindow @{action='click';label="$tag-panel";x=($p.bounds.L+92);y=($p.bounds.B-23)}
  }
  'palette' {
   K "$tag-palette" @(112)
   T "$tag-query" 'texzi.find.replaceOpen'
   K "$tag-panel" @(13)
  }
  'hotkey' {K "$tag-panel" @(17,72)}
 }
}

foreach($entry in 'menu','palette','hotkey'){
 $tag="$entry-direct"
 EnterReplace $entry $tag
 K "$tag-after-first" @(27)
 P "$tag-reset-panel" 700 415
}
EnterReplace 'menu' 'editor-focus'
P 'editor-focus-before' 970 276
K 'editor-focus-after-first' @(27)
P 'editor-focus-reset-panel' 700 415
