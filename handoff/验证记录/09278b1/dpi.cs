using System;
using System.Runtime.InteropServices;
public static class TexziDpi {
 [StructLayout(LayoutKind.Sequential)] public struct Header { public int type,size,low,high,id; }
 [StructLayout(LayoutKind.Sequential)] public struct GetDpi { public Header h; public int min,cur,max; }
 [StructLayout(LayoutKind.Sequential)] public struct SetDpi { public Header h; public int offset; }
 [DllImport("user32.dll")] static extern int GetDisplayConfigBufferSizes(uint flags,out uint paths,out uint modes);
 [DllImport("user32.dll")] static extern int QueryDisplayConfig(uint flags,ref uint paths,byte[] path,ref uint modes,byte[] mode,IntPtr topology);
 [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref GetDpi dpi);
 [DllImport("user32.dll")] static extern int DisplayConfigSetDeviceInfo(ref SetDpi dpi);
 static void Check(int code){if(code!=0)throw new System.ComponentModel.Win32Exception(code);}
 public static GetDpi Read(){
  uint n,m;Check(GetDisplayConfigBufferSizes(2,out n,out m));
  var paths=new byte[n*72];var modes=new byte[m*64];Check(QueryDisplayConfig(2,ref n,paths,ref m,modes,IntPtr.Zero));
  if(n!=1)throw new InvalidOperationException("Expected exactly one active display path.");
  var q=new GetDpi{h=new Header{type=-3,size=Marshal.SizeOf<GetDpi>(),low=BitConverter.ToInt32(paths,0),high=BitConverter.ToInt32(paths,4),id=BitConverter.ToInt32(paths,8)}};
  Check(DisplayConfigGetDeviceInfo(ref q));return q;
 }
 public static void Write(GetDpi original,int offset){
  var h=original.h;h.type=-4;h.size=Marshal.SizeOf<SetDpi>();
  var q=new SetDpi{h=h,offset=offset};Check(DisplayConfigSetDeviceInfo(ref q));
  if(Read().cur!=offset)throw new InvalidOperationException("Display offset did not apply.");
 }
}
