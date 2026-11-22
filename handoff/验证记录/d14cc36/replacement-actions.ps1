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
function Replace($tag,$value){
 $before=@(Get-Content "$run/fixtures/target.txt")
 if($before.Count -ne 1 -or $before[0] -ne 'STEAMYOLD target'){throw "Wrong starting target for $tag"}
 EnterReplace $tag $tag
 P "$tag-search" 675 519
 K "$tag-select-search" @(17,65)
 T "$tag-search-input" 'STEAMYOLD'
 P "$tag-value" 670 622
 K "$tag-select-value" @(17,65)
 T "$tag-value-input" $value
 P "$tag-result" 641 674
 P "$tag-close-panel" 700 415
 K "$tag-save" @(17,83)
 $actual=@(Get-Content "$run/fixtures/target.txt")
 P "$tag-control" 824 128
 K "$tag-control-save" @(17,83)
 $control=@(Get-Content "$run/fixtures/control.txt")
 @{entry=$tag;before=$before;expected="$value target";actual=$actual;control_expected='STEAMYOLD control';control_actual=$control;pass=($actual.Count -eq 1 -and $actual[0] -ceq "$value target" -and $control.Count -eq 1 -and $control[0] -ceq 'STEAMYOLD control')}|ConvertTo-Json|Set-Content "$run/$tag-check.json"
 P "$tag-target-back" 1021 128
}
foreach($entry in 'menu','palette','hotkey'){
 Replace $entry "$($entry.ToUpperInvariant())NEW"
 if($entry -ne 'hotkey'){
  K "$entry-undo" @(17,90)
  K "$entry-restored-save" @(17,83)
 }
}
foreach($entry in 'menu','palette','hotkey'){
 $tag="$entry-esc"
 EnterReplace $entry $tag
 P "$tag-focus-query" 675 519
 K "$tag-select-query" @(17,65)
 T "$tag-before" "ESCFOCUS$($entry.ToUpperInvariant())"
 K "$tag-after-first" @(27)
 P "$tag-reset-panel" 700 415
}
