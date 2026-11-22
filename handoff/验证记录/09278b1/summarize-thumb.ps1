$ErrorActionPreference='Stop'
$verified=Get-Content "$PSScriptRoot/ui-thumb.json" -Raw|ConvertFrom-Json
$run=$verified.run
$metrics=Get-Content "$run/02-metrics.json" -Raw|ConvertFrom-Json
$scale=$metrics.dpi/96
$originX=25
$originY=25
Add-Type -AssemblyName System.Drawing

function Read-Frame($file,$geometry,$mode){
 $bitmap=[Drawing.Bitmap]::new((Join-Path $run $file))
 try{
   $x=$geometry.sampleX-$originX
   $center=$null
   $start=-1
   for($y=[int](114*$scale);$y -le $bitmap.Height-[int](24*$scale);$y++){
    $p=$bitmap.GetPixel($x,$y)
    if($p.G -ge 43 -and $p.G -le 165){
     if($start -lt 0){$start=$y}
    }elseif($start -ge 0){
     if($y-$start -ge [int](24*$scale) -and $y-$start -le [int](55*$scale)){
      $center=$originY+($start+$y-1)/2.0
     };$start=-1
    }
   }
   $y=if($mode -match 'rich'){[int](162*$scale)}else{[int](135*$scale)}
   $region=[Drawing.Rectangle]::new([int](292*$scale),$y,[int](230*$scale),[int](155*$scale))
   $body=$bitmap.Clone($region,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
   $data=[IO.MemoryStream]::new()
   try{
    $body.Save($data,[Drawing.Imaging.ImageFormat]::Bmp)
    $digest=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($data.ToArray()))
   }finally{$body.Dispose();$data.Dispose()}
   @{thumb_y=$center;body_sha256=$digest}
 }finally{$bitmap.Dispose()}
}

$reports=foreach($mode in @('16-rich-long','17-rich-reverse','19-source-long','20-source-reverse')){
 if(!(Test-Path "$run/$mode.json")){continue}
 $g=Get-Content "$run/$mode-geometry.json" -Raw|ConvertFrom-Json
 $trace=Get-Content "$run/$mode.json" -Raw|ConvertFrom-Json
 $frames=@(foreach($shot in $trace.samples){
  $m=Read-Frame $shot.file $g $mode
  [pscustomobject]@{file=$shot.file;mouse_y=$shot.input_y;input_ms=$shot.input_ms;sample_start_ms=$shot.from_ms;sample_end_ms=$shot.end_ms;thumb_y=$m.thumb_y;body_sha256=$m.body_sha256}
 })
 $held=@($frames|Where-Object {$null -ne $_.mouse_y})
 $settled=@($frames|Where-Object {$null -eq $_.mouse_y})
 $bodyStable=@($settled|Where-Object body_sha256 -EQ $settled[-1].body_sha256)[0]
 $thumbUpdates=@(-1)
 $bodyUpdatesOnMovingThumb=@(-1)
 $lastComparable=-1
 $thumbGaps=@()
 $bodyGaps=@()
 for($i=0;$i -lt $held.Count;$i++){
  if($null -eq $held[$i].thumb_y){continue}
  if($lastComparable -lt 0 -or $held[$i].thumb_y -ne $held[$lastComparable].thumb_y){
   $previous=$thumbUpdates[-1]
   if($previous -ge 0){
    $thumbGaps+=($held[$i].sample_end_ms-$held[$previous].sample_start_ms)
   }
   $thumbUpdates+=$i
   $previousBody=$bodyUpdatesOnMovingThumb[-1]
   if($previousBody -lt 0 -or $held[$i].body_sha256 -ne $held[$previousBody].body_sha256){
    if($previousBody -ge 0){
     $bodyGaps+=($held[$i].sample_end_ms-$held[$previousBody].sample_start_ms)
    }
    $bodyUpdatesOnMovingThumb+=$i
   }
  }
  $lastComparable=$i
 }
 @{
  mode=$mode
  first_thumb=$g.y
  released_ms=$trace.released_ms
  stable_body_by_ms=[math]::Round($bodyStable.sample_end_ms-$trace.released_ms,2)
  max_sampled_gap_between_thumb_updates_ms=($thumbGaps|Measure-Object -Maximum).Maximum
  max_sampled_gap_between_body_updates_with_moving_thumb_ms=($bodyGaps|Measure-Object -Maximum).Maximum
  distinct_held_thumb_positions=@($held|Where-Object {$null -ne $_.thumb_y}|Select-Object -ExpandProperty thumb_y -Unique).Count
  distinct_held_body_positions=@($held|Select-Object -ExpandProperty body_sha256 -Unique).Count
  last_observed_thumb=($frames|Where-Object {$null -ne $_.thumb_y}|Select-Object -Last 1).thumb_y
  post_release_positions=@($settled|Where-Object {$null -ne $_.thumb_y}|Select-Object -ExpandProperty thumb_y)
  samples=$frames
 }
}
@($reports)|ConvertTo-Json -Depth 6|Tee-Object -FilePath "$PSScriptRoot/thumb-timelines.json"
