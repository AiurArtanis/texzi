param([string]$Round,[ValidateSet('trace','default','fault')][string]$Mode='trace',[switch]$FailPrepare,[string]$Exe,[string]$Commit,[string]$Hash)
$ErrorActionPreference='Stop'
if(Test-Path $Round){throw 'Preserve existing round'}
New-Item -ItemType Directory $Round|Out-Null
Copy-Item $PSCommandPath "$Round/driver.ps1"
Get-FileHash $PSCommandPath|Select-Object Hash|ConvertTo-Json|Set-Content "$Round/driver-hash.json"
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;using System.Text;using System.Runtime.InteropServices;
public class ImeOnlyProbe {
 [DllImport("user32.dll")]public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int cx,int cy,uint flags);
 [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")]public static extern IntPtr GetWindowLongPtr(IntPtr h,int index);
 [StructLayout(LayoutKind.Sequential)]public struct Point{public int X,Y;}
 [DllImport("user32.dll")]public static extern IntPtr WindowFromPoint(Point p);
 [DllImport("user32.dll")]public static extern IntPtr SendMessage(IntPtr h,uint msg,IntPtr w,IntPtr l);
 [DllImport("user32.dll")]public static extern bool EnumChildWindows(IntPtr h,Visitor v,IntPtr d);
 [DllImport("user32.dll")]public static extern int GetDlgCtrlID(IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern int GetClassName(IntPtr h,StringBuilder b,int n);
 public static IntPtr Button(IntPtr h){int count=0;IntPtr result=IntPtr.Zero;EnumChildWindows(h,(c,d)=>{var b=new StringBuilder(128);GetClassName(c,b,128);if(b.ToString()=="Button"&&IsWindowVisible(c)&&IsWindowEnabled(c)){count++;result=c;}return true;},IntPtr.Zero);return count==1?result:IntPtr.Zero;}
 [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h); public delegate bool Visitor(IntPtr h,IntPtr d);
 [StructLayout(LayoutKind.Sequential)]public struct Rect {public int L,T,R,B;}
 [StructLayout(LayoutKind.Sequential)]public struct Mouse {public int x,y;public uint data,flags,time;public UIntPtr extra;}
 [StructLayout(LayoutKind.Sequential)]public struct Keyboard {public ushort vk,scan;public uint flags,time;public UIntPtr extra;}
 [StructLayout(LayoutKind.Explicit)]public struct Union {[FieldOffset(0)]public Mouse mouse;[FieldOffset(0)]public Keyboard key;}
 [StructLayout(LayoutKind.Sequential)]public struct Input {public uint type;public Union u;}
 [DllImport("user32.dll")]static extern uint SendInput(uint n,Input[] i,int size);
 [DllImport("user32.dll")]public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")]public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")]public static extern IntPtr GetKeyboardLayout(uint thread);
 [DllImport("user32.dll")]public static extern bool EnumWindows(Visitor v,IntPtr d);
 [DllImport("user32.dll")]public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")]public static extern bool IsIconic(IntPtr h);
 [DllImport("user32.dll")]public static extern bool GetWindowRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")]public static extern bool MoveWindow(IntPtr h,int x,int y,int w,int height,bool paint);
 [DllImport("user32.dll")]public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")]public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)]static extern int GetWindowText(IntPtr h,StringBuilder b,int n);
 public static string WindowClass(IntPtr h){var b=new StringBuilder(128);GetClassName(h,b,128);return b.ToString();}
 public static int Owner(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return (int)p;}
 public static IntPtr Find(int pid,bool fault){IntPtr found=IntPtr.Zero;EnumWindows((h,d)=>{if(Owner(h)!=pid||!IsWindowVisible(h))return true;var b=new StringBuilder(256);GetWindowText(h,b,256);if((fault?b.ToString()=="nsh":b.ToString().Contains("texzi"))){found=h;return false;}return true;},IntPtr.Zero);return found;}
 public static string Layout(IntPtr h){uint p;return GetKeyboardLayout(GetWindowThreadProcessId(h,out p)).ToInt64().ToString("X16");}
 static void Send(Input i){if(SendInput(1,new[]{i},Marshal.SizeOf<Input>())!=1)throw new Exception("SendInput failed");}
 public static void Key(ushort vk,bool up){var i=new Input();i.type=1;i.u.key.vk=vk;i.u.key.flags=up?2u:0u;Send(i);}
 public static void Text(string text){foreach(char c in text){var i=new Input();i.type=1;i.u.key.scan=c;i.u.key.flags=4;Send(i);i.u.key.flags=6;Send(i);}}
 [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h,uint msg,IntPtr w,IntPtr l);
 public static void PostO(IntPtr h){if(!PostMessage(h,256,(IntPtr)79,(IntPtr)1572865)||!PostMessage(h,257,(IntPtr)79,(IntPtr)(-1072168959)))throw new Exception("PostMessage");}
 [DllImport("user32.dll")] static extern IntPtr GetMenu(IntPtr h);
 [DllImport("user32.dll")] static extern IntPtr GetSubMenu(IntPtr h,int pos);
 [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr h);
 [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr h,int pos);
 [DllImport("user32.dll")] static extern uint GetMenuState(IntPtr h,uint pos,uint flags);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetMenuString(IntPtr h,uint pos,StringBuilder s,int max,uint flags);
 static string MenuLabel(IntPtr menu,uint pos) {
  var s=new StringBuilder(256);
  GetMenuString(menu,pos,s,s.Capacity,0x400);
  return s.ToString().Split('\t')[0];
 }
 public class MenuEntry { public string Group,Name; public uint Command,State; }
 public static MenuEntry[] MenuSnapshot(IntPtr h) {
  var entries=new System.Collections.Generic.List<MenuEntry>();
  var root=GetMenu(h);
  for(uint i=0;i<GetMenuItemCount(root);i++) {
   var name=MenuLabel(root,i);
   var sub=GetSubMenu(root,(int)i);
   for(uint j=0;j<GetMenuItemCount(sub);j++) {
    var label=MenuLabel(sub,j);
    if(label.Length==0)continue;
    entries.Add(new MenuEntry {Group=name,Name=label,Command=GetMenuItemID(sub,(int)j),State=GetMenuState(sub,j,0x400)});
   }
  }
  return entries.ToArray();
 }
 public static int ActivateMenu(IntPtr hwnd,string group,string name) {
  var menu=GetMenu(hwnd);
  for(uint i=0;i<GetMenuItemCount(menu);i++) {
   if(MenuLabel(menu,i)!=group)continue;
   var sub=GetSubMenu(menu,(int)i);
   for(uint j=0;j<GetMenuItemCount(sub);j++) {
    if(MenuLabel(sub,j)!=name)continue;
    var id=GetMenuItemID(sub,(int)j);
    var state=GetMenuState(sub,j,0x400);
    if(id==0 || id>0xFFFF || (state&3)!=0)return 0;
    return PostMessage(hwnd,0x111,(IntPtr)id,IntPtr.Zero)?(int)id:0;
   }
  }
  return 0;
 }
 [StructLayout(LayoutKind.Sequential)] struct GuiThreadInfo {
  public uint size,flags;
  public IntPtr active,focus,capture,menu,move,caret;
  public Rect caretBounds;
 }
 [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GuiThreadInfo info);
 [DllImport("user32.dll")] static extern bool IsChild(IntPtr parent,IntPtr child);
 [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",CharSet=CharSet.Unicode,SetLastError=true)]
 static extern IntPtr SetText(IntPtr h,uint message,IntPtr w,string text,uint flags,uint ms,out UIntPtr result);
 [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",CharSet=CharSet.Unicode,SetLastError=true)]
 static extern IntPtr GetText(IntPtr h,uint message,IntPtr w,StringBuilder text,uint flags,uint ms,out UIntPtr result);
 public static IntPtr SetPickerFileName(IntPtr dialog,string path) {
  uint dialogPid;
  var thread=GetWindowThreadProcessId(dialog,out dialogPid);
  var info=new GuiThreadInfo {size=(uint)Marshal.SizeOf<GuiThreadInfo>()};
  if(!GetGUIThreadInfo(thread,ref info)||GetForegroundWindow()!=dialog || !IsChild(dialog,info.focus)
      || Owner(info.focus)!=(int)dialogPid || WindowClass(info.focus)!="Edit")
    throw new Exception("The owned picker file-name Edit does not have focus");
  UIntPtr accepted;
  if(SetText(info.focus,0xC,IntPtr.Zero,path,2,1800,out accepted)==IntPtr.Zero||accepted==UIntPtr.Zero)
    throw new Exception("The owned Edit rejected WM_SETTEXT");
  var buffer=new StringBuilder(path.Length+2);
  UIntPtr length;
  if(GetText(info.focus,0xD,(IntPtr)buffer.Capacity,buffer,2,1800,out length)==IntPtr.Zero||buffer.ToString()!=path)
    throw new Exception("The owned Edit value did not match the generated path");
  return info.focus;
 }
 [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr dialog,int id);
 public static IntPtr ClickFolderSelect(IntPtr dialog) {
  var button=GetDlgItem(dialog,1);
  if(GetForegroundWindow()!=dialog||button==IntPtr.Zero||!IsChild(dialog,button)||!IsWindowVisible(button)
      ||!IsWindowEnabled(button)||Owner(button)!=Owner(dialog)||WindowClass(button)!="Button")
    throw new Exception("No visible button for the owned folder picker");
  var buffer=new StringBuilder(80);
  UIntPtr length;
  if(GetText(button,0xD,(IntPtr)buffer.Capacity,buffer,2,1800,out length)==IntPtr.Zero
      || !(buffer.ToString().Replace("&","")=="选择文件夹" || buffer.ToString().Replace("&","")=="Select Folder"))
    throw new Exception("The owned dialog action is not Select Folder");
  if(!PostMessage(button,0xF5,IntPtr.Zero,IntPtr.Zero))throw new Exception("Folder confirmation did not post");
  return button;
 }
 public static IntPtr ClickFileOpen(IntPtr dialog, bool save=false) {
  var button=GetDlgItem(dialog,1);
  if(GetForegroundWindow()!=dialog||button==IntPtr.Zero||!IsChild(dialog,button)||!IsWindowVisible(button)
      ||!IsWindowEnabled(button)||Owner(button)!=Owner(dialog)||WindowClass(button)!="Button")
    throw new Exception("No visible Open button for the owned file picker");
  var buffer=new StringBuilder(80);
  UIntPtr length;
  if(GetText(button,0xD,(IntPtr)buffer.Capacity,buffer,2,1800,out length)==IntPtr.Zero)
    throw new Exception("Cannot read the owned confirmation button");
  var title=buffer.ToString().Replace("&","").Trim();
  if(save ? title!="保存" && title!="保存(S)" && title!="Save" && title!="Save(S)" : title!="打开" && title!="打开(O)" && title!="Open" && title!="Open(O)")
    throw new Exception("The owned dialog action is not Open");
  if(!PostMessage(button,0xF5,IntPtr.Zero,IntPtr.Zero))throw new Exception("File confirmation did not post");
  return button;
 }
 public static bool RequestClose(IntPtr h){return PostMessage(h,16,IntPtr.Zero,IntPtr.Zero);}
 [StructLayout(LayoutKind.Sequential)]
 struct CaptureInfo {
  public uint Size;
  public int Width, Height;
  public ushort Planes, Depth;
  public uint Compression, ImageSize;
  public int Xppm, Yppm;
  public uint Used, Important;
 }
 [DllImport("user32.dll")]static extern IntPtr GetDC(IntPtr window);
 [DllImport("user32.dll")]static extern int ReleaseDC(IntPtr window,IntPtr dc);
 [DllImport("gdi32.dll")]static extern IntPtr CreateCompatibleDC(IntPtr dc);
 [DllImport("gdi32.dll")]static extern IntPtr CreateDIBSection(IntPtr dc,ref CaptureInfo info,uint usage,out IntPtr bits,IntPtr mapping,uint offset);
 [DllImport("gdi32.dll")]static extern IntPtr SelectObject(IntPtr dc,IntPtr obj);
 [DllImport("gdi32.dll")]static extern bool DeleteObject(IntPtr obj);
 [DllImport("gdi32.dll")]static extern bool DeleteDC(IntPtr dc);
 [DllImport("gdi32.dll")]static extern bool BitBlt(IntPtr dest,int x,int y,int w,int h,IntPtr src,int sx,int sy,uint op);
 [DllImport("gdi32.dll")]static extern bool GdiFlush();
 public sealed class WheelFrame {
  public string file {get;set;}
  public double start_ms {get;set;}
  public double end_ms {get;set;}
  public int ink {get;set;}
  public int ink_bands {get;set;}
 }
 public sealed class WheelCapture {
  public WheelFrame[] samples {get;set;}
  public double first_send_end_ms {get;set;}
  public long first_send_utc_us {get;set;}
  public double? reverse_send_start_ms {get;set;}
  public double? reverse_send_end_ms {get;set;}
 }
 static void SaveWheelFrame(string path,byte[] b,int w,int h) {
  using(var writer=new System.IO.BinaryWriter(System.IO.File.Create(path))) {
   writer.Write((ushort)0x4D42);writer.Write(54+b.Length);writer.Write(0);writer.Write(54);
   writer.Write(40);writer.Write(w);writer.Write(-h);writer.Write((ushort)1);writer.Write((ushort)32);
   writer.Write(0);writer.Write(b.Length);writer.Write(0);writer.Write(0);writer.Write(0);writer.Write(0);
   writer.Write(b);
  }
 }
 public static WheelCapture CaptureWheel(IntPtr window,int x,int y,int delta,int reverse,int reverseAt,int sx,int sy,int width,int height,string directory,int dragDistance=0) {
  const int cap=180;
  var data=new byte[cap][];
  var frames=new WheelFrame[cap];
  for(int i=0;i<cap;i++) {
   data[i]=new byte[width*height*4];
   frames[i]=new WheelFrame();
  }
  var answer=new WheelCapture();
  var display=GetDC(IntPtr.Zero);
  var memory=CreateCompatibleDC(display);
  var header=new CaptureInfo{Size=40,Width=width,Height=-height,Planes=1,Depth=32};
  IntPtr pixels;
  var dib=CreateDIBSection(display,ref header,0,out pixels,IntPtr.Zero,0);
  var old=SelectObject(memory,dib);
  int count=0;
  bool pressed=false;
  try {
   if(display==IntPtr.Zero||memory==IntPtr.Zero||dib==IntPtr.Zero||pixels==IntPtr.Zero||old==IntPtr.Zero)throw new Exception("Cannot create wheel capture surface");
   var clock=System.Diagnostics.Stopwatch.StartNew();
   double dispatch=double.NaN;
   int dragStep=0;
   do {
    if(GetForegroundWindow()!=window)throw new Exception("Lost foreground during continuous sampling");
    if(double.IsNaN(dispatch) && clock.Elapsed.TotalMilliseconds>=16) {
     dispatch=clock.Elapsed.TotalMilliseconds;
     answer.first_send_utc_us=(DateTimeOffset.UtcNow.Ticks-DateTimeOffset.UnixEpoch.Ticks)/10;
     if(dragDistance!=0){
      SetCursorPos(x,y);
      var down=new Input();down.type=0;down.u.mouse.flags=2;Send(down);
      pressed=true;
     }else Wheel(x,y,delta);
     answer.first_send_end_ms=clock.Elapsed.TotalMilliseconds-dispatch;
    }
    if(!double.IsNaN(dispatch)&&reverse!=0&&!answer.reverse_send_start_ms.HasValue&&clock.Elapsed.TotalMilliseconds-dispatch>=reverseAt) {
     answer.reverse_send_start_ms=clock.Elapsed.TotalMilliseconds-dispatch;
     Wheel(x,y,reverse);
     answer.reverse_send_end_ms=clock.Elapsed.TotalMilliseconds-dispatch;
    }
    if(pressed && clock.Elapsed.TotalMilliseconds-dispatch>12*(dragStep+1)){
     if(dragStep==16){
      var up=new Input();up.type=0;up.u.mouse.flags=4;Send(up);
      pressed=false;
     }else{
      dragStep++;
      SetCursorPos(x,y+dragDistance*dragStep/16);
     }
    }
    var f=frames[count];
    f.start_ms=clock.Elapsed.TotalMilliseconds;
    if(!BitBlt(memory,0,0,width,height,display,sx,sy,0x00CC0020)||!GdiFlush())throw new Exception("Wheel capture failed");
    f.end_ms=clock.Elapsed.TotalMilliseconds;
    Marshal.Copy(pixels,data[count],0,data[count].Length);
    count++;
    if(!double.IsNaN(dispatch)&&clock.Elapsed.TotalMilliseconds-dispatch>=145)
     System.Threading.Thread.Sleep(5);
   }while(count<cap && (double.IsNaN(dispatch)||clock.Elapsed.TotalMilliseconds-dispatch<370));
   if(double.IsNaN(dispatch) || frames[count-1].end_ms-dispatch<145)throw new Exception("Incomplete timed wheel capture");
   Array.Resize(ref frames,count);
   for(int i=0;i<count;i++){
    var f=frames[i];var b=data[i];
    f.start_ms-=dispatch;f.end_ms-=dispatch;
    f.file="frame-"+i.ToString("D3")+".bmp";
    bool inBand=false;
    for(int py=0;py<height;py+=2){
     bool ink=false;
     for(int px=0;px<width;px+=2){
      int at=(py*width+px)*4;
      if(b[at]>160&&b[at+1]>160&&b[at+2]>160){ink=true;f.ink++;}
     }
     if(ink&&!inBand)f.ink_bands++;
     inBand=ink;
    }
    SaveWheelFrame(System.IO.Path.Combine(directory,f.file),b,width,height);
   }
   answer.samples=frames;
   return answer;
  }finally {
   if(pressed){var up=new Input();up.type=0;up.u.mouse.flags=4;Send(up);}
   if(old!=IntPtr.Zero&&memory!=IntPtr.Zero)SelectObject(memory,old);
   if(dib!=IntPtr.Zero)DeleteObject(dib);
   if(memory!=IntPtr.Zero)DeleteDC(memory);
   if(display!=IntPtr.Zero)ReleaseDC(IntPtr.Zero,display);
  }
 }
 public static void Wheel(int x,int y,int delta){SetCursorPos(x,y);var i=new Input();i.type=0;i.u.mouse.flags=2048;i.u.mouse.data=unchecked((uint)delta);Send(i);}
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd,out Rect r);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd,ref Point p);
 public static IntPtr FindOwnedMenu(int pid) {
  IntPtr result=IntPtr.Zero;
  EnumWindows((h,d)=>{
   if(Owner(h)!=pid||!IsWindowVisible(h)||WindowClass(h)!="#32768")return true;
   result=h;return false;
  },IntPtr.Zero);
  return result;
 }
 public static int Hit(IntPtr h,int x,int y) {
  var packed=(x&65535)|((y&65535)<<16);
  return (int)SendMessage(h,132,IntPtr.Zero,(IntPtr)packed).ToInt64();
 }
[DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr h, uint message, UIntPtr wParam, IntPtr lParam, uint flags, uint timeout, out UIntPtr result);
 public static void MouseHold(bool release){var i=new Input();i.type=0;i.u.mouse.flags=release?4u:2u;Send(i);}
 public static void Drag(int x,int y,int nx,int ny,int hold=0){SetCursorPos(x,y);var i=new Input();i.type=0;i.u.mouse.flags=2;Send(i);try{for(int s=1;s<=12;s++){SetCursorPos(x+(nx-x)*s/12,y+(ny-y)*s/12);System.Threading.Thread.Sleep(28);}System.Threading.Thread.Sleep(hold);}finally{i.u.mouse.flags=4;Send(i);}}
 public static void Click(int x,int y){SetCursorPos(x,y);var i=new Input();i.type=0;i.u.mouse.flags=2;Send(i);i.u.mouse.flags=4;Send(i);}
}
'@

[void][ImeOnlyProbe]::SetThreadDpiAwarenessContext([IntPtr](-4))
$p=$null;$h=[IntPtr]::Zero;$failure=$null;$forced=$false;$status='NOT_RUN'
function Trace($v){$v.utc=[DateTime]::UtcNow.ToString('o');$v|ConvertTo-Json -Depth 10 -Compress|Add-Content "$Round/trace.jsonl"}
function Focus {
 $before=[ImeOnlyProbe]::GetForegroundWindow();$beforePid=[ImeOnlyProbe]::Owner($before)
 $attempted=$false;$returned=$null
 if($beforePid -ne $p.Id){$attempted=$true;$returned=[ImeOnlyProbe]::SetForegroundWindow($h);Start-Sleep -Milliseconds 200}
 $after=[ImeOnlyProbe]::GetForegroundWindow();$afterPid=[ImeOnlyProbe]::Owner($after)
 Trace @{kind='foreground-check';target_hwnd=$h.ToInt64();target_pid=$p.Id;target_owner=[ImeOnlyProbe]::Owner($h);visible=[ImeOnlyProbe]::IsWindowVisible($h);minimized=[ImeOnlyProbe]::IsIconic($h);before_hwnd=$before.ToInt64();before_pid=$beforePid;set_foreground_attempted=$attempted;set_foreground_return=$returned;after_hwnd=$after.ToInt64();after_pid=$afterPid;title_read=$false}
 if($afterPid -ne $p.Id -and -not $script:captionProbeUsed){
  $script:captionProbeUsed=$true
  $rect=[ImeOnlyProbe+Rect]::new();$rectOk=[ImeOnlyProbe]::GetWindowRect($h,[ref]$rect)
  $point=[ImeOnlyProbe+Point]::new();$point.X=$rect.L+220;$point.Y=$rect.T+25
    $first=[ImeOnlyProbe]::WindowFromPoint($point);$firstPid=[ImeOnlyProbe]::Owner($first)
  if($firstPid -ne $p.Id -and [ImeOnlyProbe]::Owner($h) -eq $p.Id -and [ImeOnlyProbe]::IsWindowVisible($h) -and -not [ImeOnlyProbe]::IsIconic($h)){
   $topBefore=([ImeOnlyProbe]::GetWindowLongPtr($h,-20).ToInt64() -band 8) -ne 0
   $zReturn=$null;$topAfter=$topBefore
   if(-not $topBefore){$zReturn=[ImeOnlyProbe]::SetWindowPos($h,[IntPtr]::Zero,0,0,0,0,19);$topAfter=([ImeOnlyProbe]::GetWindowLongPtr($h,-20).ToInt64() -band 8) -ne 0}
   Trace @{kind='ordinary-zorder-assist';pid=$p.Id;hwnd=$h.ToInt64();insert_after='HWND_TOP';flags='NOMOVE|NOSIZE|NOACTIVATE';returned=$zReturn;topmost_before=$topBefore;topmost_after=$topAfter}
   if(-not $topBefore -and -not $topAfter){
    foreach($dx in @(220,420,620,820)){
     $probe=[ImeOnlyProbe+Point]::new();$probe.X=$rect.L+$dx;$probe.Y=$rect.T+25
     $candidate=[ImeOnlyProbe]::WindowFromPoint($probe);$candidatePid=[ImeOnlyProbe]::Owner($candidate);$candidateHit=$null
     if($candidatePid -eq $p.Id){$lp=($probe.X -band 65535) -bor (($probe.Y -band 65535) -shl 16);$candidateHit=[ImeOnlyProbe]::SendMessage($h,132,[IntPtr]::Zero,[IntPtr]$lp).ToInt64()}
     Trace @{kind='bounded-header-point';x=$probe.X;y=$probe.Y;point_hwnd=$candidate.ToInt64();point_pid=$candidatePid;hit=$candidateHit;target_pid=$p.Id}
     if($candidatePid -eq $p.Id -and $candidateHit -eq 2){$point=$probe;$first=$candidate;$firstPid=$candidatePid;break}
    }
   }
  }
  $hit=$null;$second=[IntPtr]::Zero;$secondPid=0;$clicked=$false
  if($rectOk -and [ImeOnlyProbe]::IsWindowVisible($h) -and -not [ImeOnlyProbe]::IsIconic($h) -and $firstPid -eq $p.Id){
   $packed=($point.X -band 65535) -bor (($point.Y -band 65535) -shl 16)
   $hit=[ImeOnlyProbe]::SendMessage($h,132,[IntPtr]::Zero,[IntPtr]$packed).ToInt64()
   if($hit -eq 2){$second=[ImeOnlyProbe]::WindowFromPoint($point);$secondPid=[ImeOnlyProbe]::Owner($second);if($secondPid -eq $p.Id){[ImeOnlyProbe]::Click($point.X,$point.Y);$clicked=$true;Start-Sleep -Milliseconds 250}}
  }
  $after=[ImeOnlyProbe]::GetForegroundWindow();$afterPid=[ImeOnlyProbe]::Owner($after)
  Trace @{kind='caption-activation-probe';target_pid=$p.Id;target_hwnd=$h.ToInt64();rect_ok=$rectOk;x=$point.X;y=$point.Y;first_hwnd=$first.ToInt64();first_pid=$firstPid;hit=$hit;second_hwnd=$second.ToInt64();second_pid=$secondPid;clicked=$clicked;after_hwnd=$after.ToInt64();after_pid=$afterPid;title_read=$false}
 }
 if($afterPid -ne $p.Id -and -not $script:ownActivationUsed){
  $script:ownActivationUsed=$true
  if([ImeOnlyProbe]::Owner($h) -ne $p.Id -or ![ImeOnlyProbe]::IsWindowVisible($h) -or [ImeOnlyProbe]::IsIconic($h)){throw 'Own window is not eligible for activation'}
  if(([ImeOnlyProbe]::GetWindowLongPtr($h,-20).ToInt64() -band 8) -ne 0){throw 'Unexpected own topmost state'}
  $raised=[ImeOnlyProbe]::SetWindowPos($h,[IntPtr](-1),0,0,0,0,19)
  $clicked=$false
  $placed=$false
  try {
   if($raised) {
    $bounds=[ImeOnlyProbe+Rect]::new()
    if(![ImeOnlyProbe]::GetWindowRect($h,[ref]$bounds)){throw 'No own window bounds'}
    foreach($dx in @(220,420,620,820)){
     $point=[ImeOnlyProbe+Point]::new();$point.X=$bounds.L+$dx;$point.Y=$bounds.T+25
     $pointHwnd=[ImeOnlyProbe]::WindowFromPoint($point)
     if($pointHwnd -ne $h -or [ImeOnlyProbe]::Owner($pointHwnd) -ne $p.Id){continue}
     $lp=($point.X -band 65535) -bor (($point.Y -band 65535) -shl 16)
     if([ImeOnlyProbe]::SendMessage($h,132,[IntPtr]::Zero,[IntPtr]$lp).ToInt64() -ne 2){continue}
     if([ImeOnlyProbe]::WindowFromPoint($point) -ne $h){continue}
     [ImeOnlyProbe]::Click($point.X,$point.Y)
     $clicked=$true
     Start-Sleep -Milliseconds 180
     break
    }
   }
  }finally{
   if($raised){$placed=[ImeOnlyProbe]::SetWindowPos($h,[IntPtr](-2),0,0,0,0,19)}
   $after=[ImeOnlyProbe]::GetForegroundWindow()
   $afterPid=[ImeOnlyProbe]::Owner($after)
   $restored=(([ImeOnlyProbe]::GetWindowLongPtr($h,-20).ToInt64() -band 8) -eq 0)
   Trace @{kind='owned-caption-activation';pid=$p.Id;hwnd=$h.ToInt64();raised=$raised;caption_clicked=$clicked;removed_topmost=$placed;normal_zorder=$restored;after_pid=$afterPid;after_hwnd=$after.ToInt64()}
   if(!$restored){throw 'Unable to restore normal test window z-order'}
  }
 }
 if($afterPid -ne $p.Id){throw 'Foreground mismatch; stop input'}
}
function Keys($keys){if(@($keys|Where-Object{$_ -in @(67,88,86)}).Count){throw 'C/X/V input prohibited'};Focus;try{foreach($k in $keys){[ImeOnlyProbe]::Key([ushort]$k,$false);Start-Sleep -Milliseconds 65}}finally{for($i=$keys.Count-1;$i -ge 0;$i--){[ImeOnlyProbe]::Key([ushort]$keys[$i],$true)}}}
function LogText {if(-not(Test-Path "$Round/stderr.log")){return ''};$s=[IO.FileStream]::new("$Round/stderr.log",[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite);$reader=[IO.StreamReader]::new($s);try{$reader.ReadToEnd()}finally{$reader.Dispose();$s.Dispose()}}
function Shot($label){Focus;$shotH=if($null -ne $shotTarget){$shotTarget}else{$h};if([ImeOnlyProbe]::Owner($shotH) -ne $p.Id){throw 'Screenshot target owner mismatch'};$rect=[ImeOnlyProbe+Rect]::new();[void][ImeOnlyProbe]::GetWindowRect($shotH,[ref]$rect);if($rect.R -le $rect.L -or $rect.B -le $rect.T){throw 'Invalid target screenshot rectangle'};$bmp=[Drawing.Bitmap]::new($rect.R-$rect.L,$rect.B-$rect.T);$g=[Drawing.Graphics]::FromImage($bmp);try{$g.CopyFromScreen($rect.L,$rect.T,0,0,$bmp.Size);$bmp.Save("$Round/$label.png")}finally{$g.Dispose();$bmp.Dispose()};Trace @{kind='screenshot';label=$label;rect=$rect;pid=$p.Id;hwnd=$h.ToInt64()}}
function Fixture {Get-ChildItem "$Round/fixtures" -File|ForEach-Object{@{path=$_.Name;length=$_.Length;hash=(Get-FileHash $_.FullName).Hash;mtime=$_.LastWriteTimeUtc.ToString('o')}}}
try {
 $Exe=(Resolve-Path -LiteralPath $Exe).Path;$actualHash=(Get-FileHash -LiteralPath $Exe).Hash
 if($actualHash -ne $Hash){throw 'Wrong exe'}
 # Only this isolated helper process and its child environment; no system setting.
 [Environment]::SetEnvironmentVariable('TEXZI_TRACE_IME',$(if($Mode -eq 'trace'){'1'}else{$null}),'Process')
 [Environment]::SetEnvironmentVariable('TEXZI_T05_FAIL_PREPARE',$(if($FailPrepare){'1'}else{$null}),'Process');New-Item -ItemType Directory "$Round/fixtures"|Out-Null
 'independent IME fixture — 中文'|Set-Content "$Round/fixtures/untouched.txt"
 Fixture|ConvertTo-Json|Set-Content "$Round/fixture-before.json"
 $launch=@{FilePath=$exe;WorkingDirectory="$Round/fixtures";RedirectStandardOutput="$Round/stdout.log";RedirectStandardError="$Round/stderr.log";PassThru=$true}
 if($Mode -eq 'fault'){$launch.ArgumentList='--tx00-fail-window-create'}
 $p=Start-Process @launch;$until=[DateTime]::UtcNow.AddSeconds(30)
 do{Start-Sleep -Milliseconds 100;$p.Refresh();if($p.HasExited){throw 'Early app exit'};$h=[ImeOnlyProbe]::Find($p.Id,($Mode -eq 'fault'))}while($h -eq 0 -and [DateTime]::UtcNow -lt $until)
 if($h -eq 0){throw 'No target window'};$status='INCOMPLETE'
 Trace @{kind='identity';commit=$Commit;ci_run=$null;local_attempt=1;attempt=1;artifact=$null;pid=$p.Id;hwnd=$h.ToInt64();hash=$actualHash;hkl=[ImeOnlyProbe]::Layout($h);mode=$Mode;clipboard='No APIs present; C/X/V prohibited'}
 if($Mode -ne 'fault'){[void][ImeOnlyProbe]::MoveWindow($h,25,25,1200,770,$true);Trace @{kind='API-assist';method='MoveWindow';rect=@(25,25,1200,770)}}
 Start-Sleep -Milliseconds 650;Shot '00-initial'
 if($Mode -eq 'trace'){
  @([ImeOnlyProbe]::MenuSnapshot($h))|ConvertTo-Json|Set-Content "$Round/native-menu-snapshot.json"
 }
 if($Mode -eq 'fault'){$b=[ImeOnlyProbe]::Button($h);if($b -eq 0){throw 'No unique enabled button'};$br=[ImeOnlyProbe+Rect]::new();[void][ImeOnlyProbe]::GetWindowRect($b,[ref]$br);@{hwnd=$b.ToInt64();id=[ImeOnlyProbe]::GetDlgCtrlID($b);rect=$br}|ConvertTo-Json|Set-Content "$Round/button.json"}
 @{pid=$p.Id;hwnd=$h.ToInt64();ready=$true}|ConvertTo-Json|Set-Content "$Round/ready.json"
 $until=[DateTime]::UtcNow.AddMinutes(5)
 while([DateTime]::UtcNow -lt $until){
  $p.Refresh();if($p.HasExited){throw 'Application exited outside an explicit close command; stop input'}
  if(-not(Test-Path "$Round/command.json")){Start-Sleep -Milliseconds 100;continue}
  $c=Get-Content "$Round/command.json" -Raw|ConvertFrom-Json;Remove-Item -LiteralPath "$Round/command.json"
  Trace @{kind='command';command=$c;pid=$p.Id;hwnd=$h.ToInt64()}
  $mark=(LogText).Length
  $shotTarget=$h; $skipShot=$false
  switch($c.action){
   'title-metrics' {
    Focus
    $rect=[ImeOnlyProbe+Rect]::new()
    $origin=[ImeOnlyProbe+Point]::new()
    if(![ImeOnlyProbe]::GetClientRect($h,[ref]$rect) -or ![ImeOnlyProbe]::ClientToScreen($h,[ref]$origin)){throw 'No client bounds'}
    @{pid=$p.Id;hwnd=$h.ToInt64();origin=$origin;width=$rect.R;height=$rect.B;dpi=[ImeOnlyProbe]::GetDpiForWindow($h);maximized=[ImeOnlyProbe]::IsZoomed($h)}|ConvertTo-Json|Set-Content "$Round/$($c.label).json"
   }
   'title-menu' {
    Focus
    $menu=[ImeOnlyProbe]::FindOwnedMenu($p.Id)
    $menuBounds=[ImeOnlyProbe+Rect]::new()
    if($menu -ne 0){[void][ImeOnlyProbe]::GetWindowRect($menu,[ref]$menuBounds)}
    @{visible=($menu -ne 0);pid=$p.Id;hwnd=$menu.ToInt64();bounds=$menuBounds}|ConvertTo-Json|Set-Content "$Round/$($c.label).json"
    if($menu -eq 0){
     if(!$c.optional){throw 'No owned native popup'}
    }else{$shotTarget=$menu}
   }
   'caption-double-click' {
    Focus
    $point=[ImeOnlyProbe+Point]::new();$point.X=$c.x;$point.Y=$c.y
    if([ImeOnlyProbe]::WindowFromPoint($point) -ne $h -or [ImeOnlyProbe]::Hit($h,$c.x,$c.y) -ne 2){throw 'Point is not an owned caption drag strip'}
    [ImeOnlyProbe]::Click($c.x,$c.y)
    Start-Sleep -Milliseconds 75
    [ImeOnlyProbe]::Click($c.x,$c.y)
   }
   'caption-control-hits'{
    Focus
    $hits=foreach($point in $c.points){[ImeOnlyProbe]::Hit($h,$point.x,$point.y)}
    @{values=@($hits);pid=$p.Id;hwnd=$h.ToInt64()}|ConvertTo-Json|Set-Content "$Round/$($c.label).json"
   }
   'invoke-menu' {
    Focus
    if([ImeOnlyProbe]::GetForegroundWindow() -ne $h){throw 'Menu target is not the main window'}
    if($c.item -in @('剪切','复制','粘贴')){throw 'Clipboard commands are outside this input round'}
    $commandId=[ImeOnlyProbe]::ActivateMenu($h,$c.group,$c.item)
    if($commandId -eq 0){throw "No enabled native menu entry: $($c.group) / $($c.item)"}
    Trace @{kind='native-menu-activation';group=$c.group;item=$c.item;wm_command=$commandId;pid=$p.Id;hwnd=$h.ToInt64();method='WM_COMMAND from current HMENU'}
    $skipShot=[bool]$c.noScreenshot
    if($c.closes){
     if(!$p.WaitForExit(10000) -or $p.ExitCode -ne 0){throw 'Main window menu close failed'}
     $status='PASS'
    }
   }
   'new-child-round' {
    $childDeadline=[DateTime]::UtcNow.AddSeconds(8)
    do {
     $newChild=@(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($p.Id) AND Name='texzi-app.exe'" | Where-Object ExecutablePath -EQ $Exe)
     if($newChild.Count -eq 1) {
      $childPid=$newChild[0].ProcessId
      $childHwnd=[ImeOnlyProbe]::Find($childPid,$false)
      if($childHwnd -ne 0){break}
     }
     Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $childDeadline)
    if($childHwnd -eq 0 -or [ImeOnlyProbe]::Owner($childHwnd) -ne $childPid){throw 'No owned new-window child'}
    $child=Get-Process -Id $childPid
    [void]$child.Handle
    if($child.Path -ne $Exe -or (Get-FileHash -LiteralPath $child.Path).Hash -ne $Hash){throw 'Wrong new-window executable'}
    if(![ImeOnlyProbe]::RequestClose($childHwnd) -or !$child.WaitForExit(10000) -or $child.ExitCode -ne 0){throw 'Owned new window did not close naturally'}
    Trace @{kind='new-window-child';pid=$childPid;hwnd=$childHwnd.ToInt64();parent_pid=$p.Id;exe_sha256=$Hash;exit_code=$child.ExitCode}
   }
   'click'{Focus;$pt=[ImeOnlyProbe+Point]::new();$pt.X=$c.x;$pt.Y=$c.y;if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($pt)) -ne $p.Id){throw 'Wrong click target'};[ImeOnlyProbe]::Click($c.x,$c.y);$skipShot=[bool]$c.noScreenshot}
   'keys'{if($c.allow_menu_key -and (($c.keys -join ',') -eq '18,86')){Focus;try{[ImeOnlyProbe]::Key(18,$false);[ImeOnlyProbe]::Key(86,$false);Start-Sleep -Milliseconds 70}finally{[ImeOnlyProbe]::Key(86,$true);[ImeOnlyProbe]::Key(18,$true)}}else{Keys $c.keys}}
   'physical'{foreach($k in $c.keys){Keys @($k);Start-Sleep -Milliseconds 150}}
   'text'{Focus;[ImeOnlyProbe]::Text($c.text)}
   'wait-dialog'{Focus;$deadline=[DateTime]::UtcNow.AddSeconds(10);do{$fg=[ImeOnlyProbe]::GetForegroundWindow();$class=[ImeOnlyProbe]::WindowClass($fg);if([ImeOnlyProbe]::Owner($fg) -eq $p.Id -and $class -eq '#32770' -and [ImeOnlyProbe]::IsWindowVisible($fg)){break};Start-Sleep -Milliseconds 120}while([DateTime]::UtcNow -lt $deadline);Trace @{kind='native-dialog';class=$class;hwnd=$fg.ToInt64();pid=[ImeOnlyProbe]::Owner($fg)};if([ImeOnlyProbe]::Owner($fg) -ne $p.Id -or $class -ne '#32770'){throw 'No visible native dialog owned by test PID'};$skipShot=$true}
   'dialog-folder-confirm' {
    Focus
    $fg=[ImeOnlyProbe]::GetForegroundWindow()
    if([ImeOnlyProbe]::Owner($fg) -ne $p.Id -or [ImeOnlyProbe]::WindowClass($fg) -ne '#32770'){throw 'Folder confirmation target is not owned'}
    $button=[ImeOnlyProbe]::ClickFolderSelect($fg)
    Trace @{kind='owned-folder-confirm';dialog=$fg.ToInt64();button=$button.ToInt64();pid=$p.Id}
    $skipShot=$true
   }
   'dialog-cancel'{Focus;$fg=[ImeOnlyProbe]::GetForegroundWindow();if([ImeOnlyProbe]::Owner($fg) -ne $p.Id -or [ImeOnlyProbe]::WindowClass($fg) -ne '#32770'){throw 'Cancel target is not owned file dialog'};Keys @(27);$skipShot=$true}
   'abort'{throw 'Orchestration requested owned test cleanup'}
   'dialog-path'{Focus;$fg=[ImeOnlyProbe]::GetForegroundWindow();if([ImeOnlyProbe]::WindowClass($fg) -ne '#32770' -or [ImeOnlyProbe]::Owner($fg) -ne $p.Id){throw 'File picker target is not owned'};$edit=[IntPtr]::Zero; for($attempt=0; $attempt -lt 3 -and $edit -eq 0; $attempt++){Keys @(18,78);Start-Sleep -Milliseconds 250;try{$edit=[ImeOnlyProbe]::SetPickerFileName($fg,[string]$c.text)}catch{if($attempt -eq 2){throw}}};Trace @{kind='owned-picker-edit';dialog=$fg.ToInt64();edit=$edit.ToInt64();pid=$p.Id;matched=$true};Start-Sleep -Milliseconds 600;$again=[ImeOnlyProbe]::SetPickerFileName($fg,[string]$c.text);if($again -ne $edit){throw 'Picker filename edit changed'};$button=[ImeOnlyProbe]::ClickFileOpen($fg,[bool]$c.save);Trace @{kind='owned-file-open';button=$button.ToInt64()};$skipShot=$true}
   'wait-main'{Focus;$deadline=[DateTime]::UtcNow.AddSeconds(10);while([ImeOnlyProbe]::GetForegroundWindow() -ne $h -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 120};if([ImeOnlyProbe]::GetForegroundWindow() -ne $h){
    $shotTarget=[ImeOnlyProbe]::GetForegroundWindow()
    $owned=[ImeOnlyProbe]::Owner($shotTarget) -eq $p.Id
    Trace @{kind='main-wait-timeout';hwnd=$shotTarget.ToInt64();own_process=$owned;class=$(if($owned){[ImeOnlyProbe]::WindowClass($shotTarget)})}
    if($owned){Shot "$($c.label)-stalled"}
    throw 'Main window did not return'
   };Trace @{kind='main-resumed';pid=$p.Id;hwnd=$h.ToInt64()}}
   'respond' {
    Focus
    $result=[UIntPtr]::Zero
    if([ImeOnlyProbe]::SendMessageTimeout($h,0,[UIntPtr]::Zero,[IntPtr]::Zero,2,9000,[ref]$result) -eq 0){throw 'Owned UI thread did not process WM_NULL in 9000 ms'}
   }
   'edge-resize' {
    Focus
    $wr=[ImeOnlyProbe+Rect]::new()
    if (![ImeOnlyProbe]::GetWindowRect($h,[ref]$wr)) { throw 'No window rectangle' }
    $y=$wr.T+200
    $x=$null
    foreach($i in 1..24) {
      $point=[ImeOnlyProbe+Point]::new();$point.X=$wr.R-$i;$point.Y=$y
      if([ImeOnlyProbe]::WindowFromPoint($point) -eq $h -and [ImeOnlyProbe]::Hit($h,$point.X,$y) -eq 11) {$x=$point.X;break}
    }
    if($null -eq $x){throw 'No owned HTRIGHT within window frame'}
    [ImeOnlyProbe]::Drag($x,$y,$x-85,$y)
   }
   'interaction-probe' {
    Focus
    $img=[Drawing.Bitmap]::new([int]$c.sw,[int]$c.sh)
    $g=[Drawing.Graphics]::FromImage($img)
    try{
        $g.CopyFromScreen($c.sx,$c.sy,0,0,$img.Size)
        $scan=@(for($yy=0;$yy -lt $img.Height;$yy+=3){for($xx=0;$xx -lt $img.Width;$xx+=3){$img.GetPixel($xx,$yy).ToArgb()}})
        $clock=[Diagnostics.Stopwatch]::StartNew()
        switch($c.kind){
          'scroll' {Trace @{kind='scroll-dispatch';label=$c.label};[ImeOnlyProbe]::Wheel($c.x,$c.y,-1440)}
          'type' {Trace @{kind='single-input-dispatch';label=$c.label};[ImeOnlyProbe]::Text('Z')}
        }
        $submitMs=$null
        $localMs=$null
        do{
            if([ImeOnlyProbe]::GetForegroundWindow() -ne $h){throw 'Foreground changed during interaction'}
            if ($c.kind -eq 'type' -and $null -eq $localMs -and (LogText) -match 'RICH_PROJECTION action=local_insert') {
                $localMs=$clock.ElapsedMilliseconds
                Trace @{kind='local-projection-observed';elapsed_ms=$localMs}
            }
            if ($c.kind -eq 'type' -and $null -eq $submitMs -and (LogText) -match 'action=submit kind=Typing previous=0 revision=1') {
                $submitMs=$clock.ElapsedMilliseconds
                Trace @{kind='shared-submit-observed';elapsed_ms=$submitMs}
            }
            $g.CopyFromScreen($c.sx,$c.sy,0,0,$img.Size)
            $index=0
            $different=0
            for($yy=0;$yy -lt $img.Height;$yy+=3){for($xx=0;$xx -lt $img.Width;$xx+=3){
                if($scan[$index] -ne $img.GetPixel($xx,$yy).ToArgb()){$different++}
                $index++
            }}
            if($different -ge $c.threshold){break}
            Start-Sleep -Milliseconds 100
        }while($clock.ElapsedMilliseconds -lt 80000)
        Trace @{kind='interaction-observation';label=$c.label;observed=($different -ge $c.threshold);elapsed_ms=$clock.ElapsedMilliseconds}; @{observed=($different -ge $c.threshold);elapsed_ms=$clock.ElapsedMilliseconds;changed_samples=$different;shared_submit_sample_ms=$submitMs;local_projection_sample_ms=$localMs} | ConvertTo-Json | Set-Content "$Round/$($c.label).json"
    }finally{$g.Dispose();$img.Dispose()}
   }
   'burst-type' {
    Focus
    $clock=[Diagnostics.Stopwatch]::StartNew()
    Trace @{kind='burst-dispatch';label=$c.label}
    [ImeOnlyProbe]::Text($c.text)
    $matched=$false
    do {
      if([ImeOnlyProbe]::GetForegroundWindow() -ne $h){throw 'Foreground changed during burst'}
      $whole=LogText
      $tail=$whole.Substring([Math]::Min($mark,$whole.Length))
      $matched=$tail -match ("action=submit kind=Typing previous=\d+ revision=\d+ bytes="+$c.bytes+"\b")
      if(!$matched){Start-Sleep -Milliseconds 100}
    } while(!$matched -and $clock.ElapsedMilliseconds -lt 90000)
    Trace @{kind='burst-result';matched=$matched;elapsed_ms=$clock.ElapsedMilliseconds}
    if(!$matched){throw 'Shared document did not receive all burst characters'}
    Start-Sleep -Milliseconds 650
   }
   'undo-observe' {
    Focus
    $clock=[Diagnostics.Stopwatch]::StartNew()
    Trace @{kind='undo-dispatch';label=$c.label}
    Keys @(17,90)
    $match=$null
    do {
      if([ImeOnlyProbe]::GetForegroundWindow() -ne $h){throw 'Foreground changed during undo'}
      $whole=LogText
      $tail=$whole.Substring([Math]::Min($mark,$whole.Length))
      $match=[regex]::Match($tail, 'action=undo previous=\d+ revision=\d+ bytes=(?<bytes>\d+)')
      if(!$match.Success){Start-Sleep -Milliseconds 100}
    } while(!$match.Success -and $clock.ElapsedMilliseconds -lt 60000)
    if(!$match.Success){throw 'No shared-document undo observation'}
    @{bytes=[long]$match.Groups['bytes'].Value;elapsed_ms=$clock.ElapsedMilliseconds} | ConvertTo-Json | Set-Content "$Round/$($c.label).json"
    Start-Sleep -Milliseconds 650
   }
   'paint-probe' {
    Focus
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $body=[Drawing.Bitmap]::new([int]$c.bw,[int]$c.bh)
    $bodyGraphics=[Drawing.Graphics]::FromImage($body)
    $bodyGraphics.CopyFromScreen($c.bx,$c.by,0,0,$body.Size)
    $bodyStart=@(for($y=0;$y -lt $body.Height;$y+=4){for($x=0;$x -lt $body.Width;$x+=4){$body.GetPixel($x,$y).ToArgb()}})
    $bitmap=[Drawing.Bitmap]::new([int]$c.sw,[int]$c.sh)
    $graphics=[Drawing.Graphics]::FromImage($bitmap)
    if ($c.rich -and $env:TEXZI_CPU_SAMPLER) {
        $profile=Start-Process -FilePath $env:TEXZI_CPU_SAMPLER -ArgumentList @($h.ToInt64(),$p.Id,$env:TEXZI_PDB_DIRECTORY) -RedirectStandardOutput "$Round/cpu.tsv" -RedirectStandardError "$Round/cpu.stderr" -WindowStyle Hidden -PassThru
        Trace @{kind='cpu-sampler';sampler_pid=$profile.Id;target_pid=$p.Id}
    }
    $timer.Restart()
    Trace @{kind='switch-click';want_rich=$c.rich;label=$c.label}
    [ImeOnlyProbe]::Click($c.x,$c.y)
    $observed=$false
    $peak=0
    try {
      do {
        if([ImeOnlyProbe]::GetForegroundWindow() -ne $h){throw 'Foreground changed during paint wait'}
        $graphics.CopyFromScreen($c.sx,$c.sy,0,0,$bitmap.Size)
        $ink=0
        for($y=0;$y -lt $bitmap.Height;$y+=2){
          for($x=0;$x -lt $bitmap.Width;$x+=2){
            $color=$bitmap.GetPixel($x,$y)
            if($color.R -gt 160 -and $color.G -gt 160 -and $color.B -gt 160){$ink++}
          }
        }
        $peak=[Math]::Max($peak,$ink)
        $observed=if($c.rich){$ink -ge 40}else{$ink -lt 40}
        if($observed -and $c.rich) {
            $bodyGraphics.CopyFromScreen($c.bx,$c.by,0,0,$body.Size)
            $changed=0
            $inkBody=0
            $index=0
            for($y=0;$y -lt $body.Height;$y+=4) {
              for($x=0;$x -lt $body.Width;$x+=4) {
                $color=$body.GetPixel($x,$y)
                if($color.ToArgb() -ne $bodyStart[$index]){$changed++}
                if($color.R -gt 160 -and $color.G -gt 160 -and $color.B -gt 160){$inkBody++}
                $index++
              }
            }
            $observed=$changed -ge 100 -and $inkBody -ge 30
        }
        if($observed){Trace @{kind='body-observed';want_rich=$c.rich;label=$c.label;elapsed_ms=$timer.Elapsed.TotalMilliseconds};break}
        Start-Sleep -Milliseconds 100
      } while($timer.ElapsedMilliseconds -lt 90000)
    } finally {$graphics.Dispose();$bitmap.Dispose();$bodyGraphics.Dispose();$body.Dispose()}
    [ordered]@{observed=$observed;want_rich=$c.rich;elapsed_ms=$timer.ElapsedMilliseconds;light_pixels=$ink;peak=$peak;body_pixels_changed=$changed;body_light_pixels=$inkBody} | ConvertTo-Json | Set-Content "$Round/$($c.label).json"
    $skipShot=!$observed
   }
   'drag' {
    Focus
    $point=[ImeOnlyProbe+Point]::new();$point.X=$c.x;$point.Y=$c.y
    if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($point)) -ne $p.Id){throw 'Wrong drag target'}
    $point.X=$c.nx;$point.Y=$c.ny
    if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($point)) -ne $p.Id){throw 'Wrong drag end target'}
    [ImeOnlyProbe]::Drag($c.x,$c.y,$c.nx,$c.ny,[int]$c.hold)
   }
   'drop-ole' {
    $source=[IntPtr]$c.sourceHwnd
    $point=[ImeOnlyProbe+Point]::new();$point.X=$c.x;$point.Y=$c.y
    $atSource=[ImeOnlyProbe]::WindowFromPoint($point)
    Trace @{kind='ole-geometry';source_owner=[ImeOnlyProbe]::Owner($source);source_visible=[ImeOnlyProbe]::IsWindowVisible($source);point_owner=[ImeOnlyProbe]::Owner($atSource)}
    if([ImeOnlyProbe]::Owner($source) -ne $c.sourcePid -or [ImeOnlyProbe]::Owner($atSource) -ne $c.sourcePid){throw 'Wrong OLE source'}
    $point.X=$c.nx;$point.Y=$c.ny
    if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($point)) -ne $p.Id){throw 'Wrong OLE destination'}
    [void][ImeOnlyProbe]::SetForegroundWindow($source)
    Start-Sleep -Milliseconds 100
    if([ImeOnlyProbe]::GetForegroundWindow() -ne $source){throw 'OLE source did not become foreground'}
    [ImeOnlyProbe]::Drag($c.x,$c.y,$c.nx,$c.ny,150)
    Start-Sleep -Milliseconds 600
   }
   'switch-samples' {
    Focus
    $point=[ImeOnlyProbe+Point]::new();$point.X=$c.x;$point.Y=$c.y
    if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($point)) -ne $p.Id){throw 'Wrong switch target'}
    $modeClock=[Diagnostics.Stopwatch]::StartNew()
    [ImeOnlyProbe]::Click($c.x,$c.y)
    $samples=foreach($mark in $c.marks){
     while($modeClock.ElapsedMilliseconds -lt $mark){Start-Sleep -Milliseconds 20}
     $from=$modeClock.Elapsed.TotalMilliseconds
     Shot "$($c.label)-$mark"
     @{requested_ms=$mark;start_ms=$from;end_ms=$modeClock.Elapsed.TotalMilliseconds}
    }
    @($samples)|ConvertTo-Json|Set-Content "$Round/$($c.label).json"
    $skipShot=$true
   }
   'detect-thumb' {
    Focus
    $point=[ImeOnlyProbe+Point]::new();$point.X=$c.x
    $height=$c.bottom-$c.top
    $bitmap=[Drawing.Bitmap]::new(1,$height)
    $graphics=[Drawing.Graphics]::FromImage($bitmap)
    $found=$null
    try {
     foreach($candidate in $c.guesses) {
      $point.Y=$candidate
      if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($point)) -ne $p.Id){continue}
      [void][ImeOnlyProbe]::SetCursorPos($c.x-26,$candidate)
      Start-Sleep -Milliseconds 25
      [void][ImeOnlyProbe]::SetCursorPos($c.x,$candidate)
      Start-Sleep -Milliseconds 135
      $graphics.CopyFromScreen($c.sampleX,$c.top,0,0,$bitmap.Size)
      $start=-1
      for($row=0;$row -le $height;$row++){
       $bright=$row -lt $height -and ($bitmap.GetPixel(0,$row).G -in 105..165)
       if($bright){
        if($start -lt 0){$start=$row}
       }elseif($start -ge 0){
        if($row-$start -ge $c.minLength -and $candidate -ge $c.top+$start -and $candidate -lt $c.top+$row){
         $found=@{x=$c.x;sampleX=$c.sampleX;top=$c.top+$start;bottom=$c.top+$row-1;y=[int]($c.top+($row+$start)/2)}
         break
        };$start=-1
       }
      }
      if($found){break}
     }
    }finally{$graphics.Dispose();$bitmap.Dispose()}
    if(!$found){throw "Native thumb not found for $($c.label)"}
    $found|ConvertTo-Json|Set-Content "$Round/$($c.label).json"
   }
   'thumb-held' {
    Focus
    $point=[ImeOnlyProbe+Point]::new();$point.X=$c.x;$point.Y=$c.y
    if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($point)) -ne $p.Id){throw 'Wrong thumb target'}
    [void][ImeOnlyProbe]::SetCursorPos($c.x,$c.y)
    Start-Sleep -Milliseconds 200
    Shot "$($c.label)-pre"
    $sw=[Diagnostics.Stopwatch]::StartNew()
    $sample=0
    $timeline=[Collections.Generic.List[object]]::new()
    [ImeOnlyProbe]::MouseHold($false)
    try {
     foreach($y in $c.positions){
      $sample++
      if([ImeOnlyProbe]::GetForegroundWindow() -ne $h){throw 'Thumb lost foreground'}
      $inputAt=$sw.Elapsed.TotalMilliseconds
      [void][ImeOnlyProbe]::SetCursorPos($c.x,$y)
      Start-Sleep -Milliseconds 30
      $filename="$($c.label)-held-$('{0:d2}' -f $sample)"
      $from=$sw.Elapsed.TotalMilliseconds
      Shot $filename
      $timeline.Add(@{input_y=$y;input_ms=$inputAt;from_ms=$from;end_ms=$sw.Elapsed.TotalMilliseconds;file="$filename.png"})
     }
    } finally {[ImeOnlyProbe]::MouseHold($true)}
    $releasedAt=$sw.Elapsed.TotalMilliseconds
    foreach($delay in @(30,80,200,450,900)){
     while($sw.Elapsed.TotalMilliseconds -lt $releasedAt+$delay){Start-Sleep -Milliseconds 5}
     $filename="$($c.label)-released-$delay"
     $from=$sw.Elapsed.TotalMilliseconds
     Shot $filename
     $timeline.Add(@{from_ms=$from;end_ms=$sw.Elapsed.TotalMilliseconds;file="$filename.png"})
    }
    @{input_x=$c.x;input_start_y=$c.y;released_ms=$releasedAt;samples=@($timeline.ToArray())}|ConvertTo-Json -Depth 5|Set-Content "$Round/$($c.label).json"
    $skipShot=$true
   }
   'copy-probe' {
    Focus
    if(!$env:TEXZI_DIAGNOSTIC_COPY_PAYLOAD -or !$env:TEXZI_DIAGNOSTIC_COPY_PAYLOAD.StartsWith($Round,[StringComparison]::OrdinalIgnoreCase)){throw 'No isolated diagnostic copy path'}
    [ImeOnlyProbe]::Key(17,$false)
    try{[ImeOnlyProbe]::Key(67,$false);[ImeOnlyProbe]::Key(67,$true)}finally{[ImeOnlyProbe]::Key(17,$true)}
   }
   'hover-scroll' {
    Focus
    $pt=[ImeOnlyProbe+Point]::new();$pt.X=$c.x;$pt.Y=$c.y
    if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($pt)) -ne $p.Id){throw 'Wrong hover target'}
    if(![ImeOnlyProbe]::SetCursorPos($c.x,$c.y)){throw 'Cannot move to prospective thumb'}
   }
   'wheel-frames' {
    Focus
    $pt=[ImeOnlyProbe+Point]::new();$pt.X=$c.x;$pt.Y=$c.y
    if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($pt)) -ne $p.Id){throw 'Wrong wheel target'}
    $capture=Join-Path $Round $c.label
    [IO.Directory]::CreateDirectory($capture)|Out-Null
    $rows=@([regex]::Matches((LogText),'TEXZI_RICH_ROW_CREATED index=(\d+)')|ForEach-Object{[int]$_.Groups[1].Value}|Sort-Object -Unique)
    if(!$rows.Count -and !$c.dragDistance){throw 'Packaged EXE did not report any created rich rows'}
    $data=[ImeOnlyProbe]::CaptureWheel($h,$c.x,$c.y,$c.delta,[int]$c.reverse,[int]$c.reverseAt,$c.sx,$c.sy,$c.sw,$c.sh,$capture,[int]$c.dragDistance)
    [ordered]@{initial_created=$rows;delta=$c.delta;reverse=$c.reverse;first_send_end_ms=$data.first_send_end_ms;first_send_utc_us=$data.first_send_utc_us;reverse_send_start_ms=$data.reverse_send_start_ms;reverse_send_end_ms=$data.reverse_send_end_ms;samples=$data.samples;roi=@{x=$c.sx;y=$c.sy;width=$c.sw;height=$c.sh}}|ConvertTo-Json -Depth 5|Set-Content "$capture/frames.json"
   }
   'wheel'{Focus;$pt=[ImeOnlyProbe+Point]::new();$pt.X=$c.x;$pt.Y=$c.y;if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($pt)) -ne $p.Id){throw 'Wrong wheel target'};[ImeOnlyProbe]::Wheel($c.x,$c.y,$c.delta)}
   'resize'{if(-not [ImeOnlyProbe]::MoveWindow($h,$c.x,$c.y,$c.w,$c.h,$true)){throw 'MoveWindow failed'}}
   'physical-open'{Focus;[ImeOnlyProbe]::Key(17,$false);try{Start-Sleep -Milliseconds 100;[ImeOnlyProbe]::Key(79,$false);Start-Sleep -Milliseconds 100;[ImeOnlyProbe]::Key(79,$true)}finally{[ImeOnlyProbe]::Key(17,$true)};Start-Sleep -Milliseconds 850;$fg=[ImeOnlyProbe]::GetForegroundWindow();$fgPid=[ImeOnlyProbe]::Owner($fg);Trace @{kind='physical-open-result';foreground=$fg.ToInt64();owner=$fgPid;process=(Get-Process -Id $fgPid -ErrorAction SilentlyContinue).ProcessName};if($fgPid -ne $p.Id){@{label=$c.label;complete=$true}|ConvertTo-Json|Set-Content "$Round/ack-$($c.label).json";continue};Trace @{kind="dialog-identity";hwnd=$fg.ToInt64();class=[ImeOnlyProbe]::WindowClass($fg);visible=[ImeOnlyProbe]::IsWindowVisible($fg);pid=$fgPid};$skipShot=$true}
   'capture'{if($c.foreground){$shotTarget=[ImeOnlyProbe]::GetForegroundWindow()}}
   'capture-owned'{$shotTarget=[IntPtr]$c.hwnd;if([ImeOnlyProbe]::Owner($shotTarget) -ne $p.Id){throw 'Wrong child owner'}}
   'request-close'{Keys @(18,115)}
   'keys-close'{Keys $c.keys;if(-not $p.WaitForExit(10000)){throw 'Close timeout'};if($p.ExitCode -ne 0){throw 'Nonzero normal exit'};$status='PASS'}
   'discard-close'{Focus;[ImeOnlyProbe]::Click($c.x,$c.y);if(-not $p.WaitForExit(10000)){throw 'Discard close timeout'};if($p.ExitCode -ne 0){throw 'Discard close nonzero'};$status='PASS'}
   'close'{Keys @(18,115);if(-not $p.WaitForExit(60000)){throw 'Close timeout'};if($p.ExitCode -ne 0){throw 'Nonzero normal exit'};$status='PASS'}
   'confirm-fault'{if($Mode -ne 'fault'){throw 'Fault mode required'};Focus;[ImeOnlyProbe]::Click($c.x,$c.y);if(-not $p.WaitForExit(10000)){throw 'Fault confirm timeout'};if($p.ExitCode -ne 1){throw 'Wrong fault exit'};$status='PASS'}
   default{throw 'Unknown action'}
  }
  if($c.pattern){$deadline=[DateTime]::UtcNow.AddSeconds(8);do{$log=LogText;$tail=$log.Substring([Math]::Min($mark,$log.Length));if($tail -match $c.pattern){break};Start-Sleep -Milliseconds 80}while([DateTime]::UtcNow -lt $deadline);$matched=$tail -match $c.pattern;Trace @{kind='diagnostic-assert';label=$c.label;pattern=$c.pattern;matched=$matched;from=$mark};if(-not $matched){throw "Diagnostic missing: $($c.label)"}}
  if($null -ne $c.submits){$actual=[regex]::Matches((LogText),'TX04_IME_OWNER action=submit').Count;Trace @{kind='submit-count';label=$c.label;expected=$c.submits;actual=$actual};if($actual -ne $c.submits){throw 'Submit count mismatch'}}
  if($status -eq 'PASS'){Trace @{kind='owned-exit';exit=$p.ExitCode};break}
  Start-Sleep -Milliseconds 350;if(!$skipShot){Shot $c.label}
  @{label=$c.label;complete=$true}|ConvertTo-Json|Set-Content "$Round/ack-$($c.label).json"
 }
 if($status -ne 'PASS'){throw 'Command host timeout'}
}catch{$failure=$_.ToString();Trace @{kind='failure';error=$failure}}finally{
 if($null -ne $p){$p.Refresh();if(-not $p.HasExited){$forced=$true;Stop-Process -Id $p.Id;$p.WaitForExit()}}
 if(Test-Path "$Round/fixtures"){Fixture|ConvertTo-Json|Set-Content "$Round/fixture-after.json"}
 @{status=$status;failure=$failure;pid=$(if($null -ne $p){$p.Id}else{$null});hwnd=$h.ToInt64();exit=$(if($null -ne $p -and $p.HasExited){$p.ExitCode}else{$null});forced=$forced;residual=$(if($null -ne $p){[bool](Get-Process -Id $p.Id -ErrorAction SilentlyContinue)}else{$false});clipboard_access=$false}|ConvertTo-Json|Set-Content "$Round/result.json"
}


























if ($status -ne "PASS") { exit 1 }
































