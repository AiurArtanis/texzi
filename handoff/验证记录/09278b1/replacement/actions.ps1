function K($label,$keys){Invoke-TexziWindow @{action='keys';label=$label;keys=$keys}}
function P($label,$x,$y){Invoke-TexziWindow @{action='click';label=$label;x=25+$x;y=25+$y}}
function T($label,$s){Invoke-TexziWindow @{action='text';label=$label;text=$s}}
function Replace($tag,$value,[switch]$Reset){
 P "$tag-search" 675 519
 if($Reset){K "$tag-select-search" @(17,65)}
 T "$tag-search-input" 'STEAMYOLD'
 P "$tag-value" 670 622
 if($Reset){K "$tag-select-value" @(17,65)}
 T "$tag-value-input" $value
 P "$tag-result" 641 674
 P "$tag-close-panel" 700 415
 K "$tag-save" @(17,83)
 $actual=(Get-Content "$run/fixtures/target.txt" -Raw).Trim()
 P "$tag-control" 824 128
 K "$tag-control-save" @(17,83)
 $control=(Get-Content "$run/fixtures/control.txt" -Raw).Trim()
 @{entry=$tag;expected="$value target";actual=$actual;control_expected='STEAMYOLD control';control_actual=$control;pass=($actual -eq "$value target" -and $control -eq 'STEAMYOLD control')}|ConvertTo-Json|Set-Content "$run/$tag-check.json"
 P "$tag-target-back" 1021 128
}
Replace 'palette' 'PALETTENEW'
K 'restore-target' @(17,90)
K 'restore-target-save' @(17,83)
K 'hotkey-panel' @(17,72)
Replace 'hotkey' 'HOTKEYNEW' -Reset
K 'light-palette' @(112)
T 'light-query' 'texzi.appearance.light'
K 'light-on' @(13)
K 'light-edit-menu' @(18,69)
Invoke-TexziWindow @{action='title-menu';label='light-edit-popup'}
K 'light-menu-close' @(27)
