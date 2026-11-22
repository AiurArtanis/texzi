param([string]$Disposition)
$ErrorActionPreference='Stop'
$rows=@(Get-Content -LiteralPath $Disposition | Where-Object {$_ -match '^\| texzi\.'} | ForEach-Object {
 $f=$_ -split '\|'
 [pscustomobject]@{id=$f[1].Trim();disposition=$f[4].Trim()}
})
$native=(git show d14cc36:crates/texzi-app/src/native_menu.rs) -join "`n"
if($LASTEXITCODE){throw 'Unable to resolve native_menu.rs'}
$registered=(git show d14cc36:docs/design-v1/data/commands.json|Out-String|ConvertFrom-Json).commands.id
if($LASTEXITCODE){throw 'Unable to resolve commands.json'}
function Ids($source) { @([regex]::Matches($source,'"(texzi\.[^"]+)"')|ForEach-Object {$_.Groups[1].Value}) }
$extra=Ids ([regex]::Match($native,'(?s)for \(id, name\) in \[(.*?)\]').Groups[1].Value)
$removed=Ids ([regex]::Match($native,'(?s)const REMOVED_COMMANDS: &\[&str\] = &\[(.*?)\]').Groups[1].Value)
$search=@($registered)+@($extra)|Where-Object {$_ -notin $removed}
$counts=@{}
$actualMenu=@()
$diff=@()
foreach($pair in @(@('File','文件'),@('Edit','编辑'),@('Appearance','外观'))){
 $block=[regex]::Match($native,('(?s)Self::'+$pair[0]+' => &\[(.*?)\]')).Groups[1].Value
 $ids=@(Ids $block)
 $expected=@($rows|Where-Object disposition -EQ "留在菜单（$($pair[1])）"|ForEach-Object id)
 $counts[$pair[1]]=$ids.Count
 $actualMenu+=$ids
 $diff+=@(Compare-Object $expected $ids | Select-Object @{n='section';e={$pair[0]}},InputObject,SideIndicator)
}
$removeExpected=@($rows | Where-Object disposition -Like '彻底移除*' | ForEach-Object id)
$onlySearchExpected=@($rows | Where-Object disposition -Like '仅留搜索*' | ForEach-Object id)
$onlySearchActual=@($rows.id|Where-Object {$_ -in $search -and $_ -notin $actualMenu})
$diff+=@(Compare-Object $removeExpected $removed | Select-Object @{n='section';e={'remove'}},InputObject,SideIndicator)
$diff+=@(Compare-Object $onlySearchExpected $onlySearchActual | Select-Object @{n='section';e={'search-only'}},InputObject,SideIndicator)
$menuMissingFromSearch=@($actualMenu|Where-Object {$_ -notin $search})
$pass=($rows.Count -eq 120 -and ($rows.id|Sort-Object -Unique).Count -eq 120 -and $diff.Count -eq 0 -and $menuMissingFromSearch.Count -eq 0 -and $removed.Count -eq 12 -and $onlySearchActual.Count -eq 90 -and $actualMenu.Count -eq 18)
@{commit='d14cc36';disposition_sha256=(Get-FileHash -LiteralPath $Disposition).Hash;total=$rows.Count;menu=$actualMenu.Count;menu_groups=$counts;search_only=$onlySearchActual.Count;removed=$removed.Count;differences=$diff;menu_missing_from_search=$menuMissingFromSearch;pass=$pass}|ConvertTo-Json -Depth 7|Set-Content "$PSScriptRoot/disposition.json"
if(!$pass){exit 1}
