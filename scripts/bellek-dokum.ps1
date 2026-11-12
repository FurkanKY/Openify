param(
    # Uygulamaya verilecek ek ortam değişkenleri: @{K="V"} ya da "K=V;K=V"
    # (ör. -Env "OPENIFY_PROF_HIDE=images;OPENIFY_TEST_OFFLINE=1").
    $Env = $null,
    # Ölçülecek exe (varsayılan: target\release\openify.exe). Geçici bir
    # klasöre kopyalanır, böylece derleme çıktısı kilitlenmez.
    [string]$Exe = "",
    [string]$Label = "",
    # Zaman çizelgesi noktaları (sn, işlem başlangıcından): özel bayt bu
    # anlarda okunur ("0.5;1;2" biçiminde, noktalı). Son nokta aynı zamanda döküm (-Detay) anıdır.
    [string]$At = "0.5;1;2;4;8;12",
    # Son noktada ayrıntılı döküm: bölge türleri, yığın (heap), iş parçacığı
    # yığıtları, en büyük özel bölgeler, modül bazında bellek.
    [switch]$Detay,
    # Debug günlüğündeki "aşama" satırlarını da yaz (RUST_LOG=openify=debug).
    [switch]$Gunluk,
    # Dökümde en büyük özel bölgelerin içine bak: birkaç ofsette hex ve metin örnekleri.
    [switch]$Bak,
    # Sonuçları buraya CSV olarak ekle: etiket,deneme,t,ozelMB,calismaMB
    [string]$Csv = "",
    [int]$Repeat = 1,
    # Son zaman noktasından sonra bellek-olcumu.ps1 gibi kaydır (4 tur: liste
    # aşağı, kenar çubuğu aşağı-yukarı; kapaklar yüklenir), 4 sn bekle, "kaydir"
    # noktasını oku; -Detay ile döküm bu anda alınır.
    [switch]$Kaydir,
    # Dökümde büyük ayırmaları kimin tuttuğunu ara (işaretçi taraması; birkaç sn sürer).
    [switch]$Referans
)
# Bellek dökümü: Openify'ı 2. monitörde, sessiz ve odak çalmadan açar
# (bellek-olcumu.ps1 / kare-olcumu.ps1 ile aynı düzen: OPENIFY_TEST_DISPLAY,
# ses 0, sahte çalma, odak geri verilir), ilk saniyelerde belleği zaman
# çizelgesi olarak okur ve istenirse son anda işlemin adres uzayını döker:
#   - VirtualQueryEx: işlenmiş (committed) bellek türüne göre (özel / görüntü /
#     eşlenmiş) ve korumaya göre; QueryWorkingSetEx ile paylaşılan sayfalar
#   - NT heap'leri: RtlQueryProcessDebugInformation(PDI_HEAPS) ile her heap'in
#     ayrılan (allocated) ve işlenen (committed) baytı. Rust'ın ayırıcısı
#     varsayılan olarak süreç heap'ini kullanır, yani "Rust heap'i" budur.
#   - İş parçacığı yığıtları: TEB'den StackBase/StackLimit, adı GetThreadDescription'dan
#   - Kalan özel bellek (VirtualAlloc: sürücü / GPU / DirectX / gpui) ve en büyük bölgeler
#   - Dosya eşlemeleri (yazı tipi dosyaları, DLL'ler) modül bazında
# Gerçek şarkı ÇALMAZ. Önce: (derleme zaten var) — bu betik cargo çalıştırmaz.
#   powershell -File scripts\bellek-dokum.ps1 -Label temel -Detay
#   powershell -File scripts\bellek-dokum.ps1 -Env "OPENIFY_TEST_OFFLINE=1" -Label cevrimdisi
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
[System.Threading.Thread]::CurrentThread.CurrentCulture = [System.Globalization.CultureInfo]::InvariantCulture
$atList = @(($At -split "[;,]") | Where-Object { $_.Trim() } | ForEach-Object { [double]::Parse($_.Trim(), [System.Globalization.CultureInfo]::InvariantCulture) })
$srcExe = if ($Exe) { $Exe } else { Join-Path $PSScriptRoot '..\target\release\openify.exe' }
if (-not (Test-Path $srcExe)) { throw "exe bulunamadı: $srcExe" }

$extraEnv = [ordered]@{}
if ($Env -is [System.Collections.IDictionary]) {
    foreach ($k in $Env.Keys) { $extraEnv[[string]$k] = [string]$Env[$k] }
} elseif ($Env) {
    foreach ($pair in ([string]$Env -split ';')) {
        if (-not $pair.Trim()) { continue }
        $i = $pair.IndexOf('=')
        if ($i -lt 1) { throw "-Env biçimi K=V;K=V olmalı: '$pair'" }
        $extraEnv[$pair.Substring(0, $i).Trim()] = $pair.Substring($i + 1)
    }
}
if ($Gunluk) { $extraEnv['RUST_LOG'] = 'openify=debug' }

$settings = Join-Path $env:TEMP ("openify-ayar-" + [guid]::NewGuid().ToString('N').Substring(0,8) + ".json")  # bu çalıştırmaya özel; gerçek settings.json'a dokunulmaz

if (-not ('Dokum' -as [type])) {
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Diagnostics;

public class Bolge {
  public long Taban, AyirmaTabani, Boyut; public uint Tur, Koruma; public string Ad;
  public long Sayfa, Paylasilan;   // çalışma kümesindeki sayfa / paylaşılan sayfa sayısı
}
public class Yigit { public int Tid; public string Ad; public long Taban, Limit, AyirmaTabani, Islenen; }
public class HeapBilgi { public long Taban; public uint Bayrak; public long Ayrilan, Islenen; public long AyirmaTabani; }

public static class Dokum {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static bool Restore(IntPtr target) {
    IntPtr cur = GetForegroundWindow();
    if (cur == target) return true;
    uint me = GetCurrentThreadId();
    uint them = GetWindowThreadProcessId(cur, IntPtr.Zero);
    AttachThreadInput(me, them, true);
    bool ok = SetForegroundWindow(target);
    AttachThreadInput(me, them, false);
    return ok;
  }

  [StructLayout(LayoutKind.Sequential)]
  struct MBI { public IntPtr BaseAddress, AllocationBase; public uint AllocationProtect; public ushort PartitionId; public IntPtr RegionSize; public uint State, Protect, Type; }
  [StructLayout(LayoutKind.Sequential)]
  struct WSEX { public IntPtr Adres; public UIntPtr Bayrak; }
  [StructLayout(LayoutKind.Sequential)]
  struct PBI { public IntPtr Exit, Peb, Aff, Prio, Pid, Parent; }

  [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint acc, bool inh, int pid);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint acc, bool inh, uint tid);
  [DllImport("kernel32.dll")] static extern int GetThreadDescription(IntPtr h, out IntPtr name);
  [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
  [DllImport("kernel32.dll")] static extern int VirtualQueryEx(IntPtr h, IntPtr addr, out MBI mbi, int len);
  [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr h, IntPtr addr, byte[] buf, int n, out IntPtr read);
  [DllImport("psapi.dll", CharSet=CharSet.Unicode)] static extern uint GetMappedFileNameW(IntPtr h, IntPtr addr, StringBuilder sb, uint n);
  [DllImport("psapi.dll", SetLastError=true)] static extern bool QueryWorkingSetEx(IntPtr h, [In, Out] WSEX[] ws, int cb);
  [DllImport("ntdll.dll")] static extern int NtQueryInformationProcess(IntPtr h, int cls, out PBI pbi, int len, out int ret);
  [DllImport("ntdll.dll")] static extern int NtQueryInformationThread(IntPtr h, int cls, byte[] buf, int len, out int ret);
  [DllImport("ntdll.dll")] static extern IntPtr RtlCreateQueryDebugBuffer(uint size, bool eventPair);
  [DllImport("ntdll.dll")] static extern int RtlQueryProcessDebugInformation(IntPtr pid, uint flags, IntPtr buf);
  [DllImport("ntdll.dll")] static extern int RtlDestroyQueryDebugBuffer(IntPtr buf);

  const uint PROCESS_QUERY_INFORMATION = 0x0400, PROCESS_VM_READ = 0x0010, PROCESS_QUERY_LIMITED = 0x1000;
  const uint MEM_COMMIT = 0x1000;

  public static IntPtr Ac(int pid) { return OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid); }
  public static void Kapat(IntPtr h) { CloseHandle(h); }

  // Adres uzayını dolaşır; yalnız işlenmiş (committed) bölgeleri döndürür.
  public static List<Bolge> Gez(IntPtr h) {
    var liste = new List<Bolge>();
    long adr = 0; var ad = new Dictionary<long, string>();
    MBI m; int sz = Marshal.SizeOf(typeof(MBI));
    while (adr >= 0 && adr < 0x7FFFFFFF0000L && VirtualQueryEx(h, (IntPtr)adr, out m, sz) != 0) {
      long taban = (long)m.BaseAddress, boy = (long)m.RegionSize;
      if (m.State == MEM_COMMIT) {
        var b = new Bolge { Taban = taban, AyirmaTabani = (long)m.AllocationBase, Boyut = boy, Tur = m.Type, Koruma = m.Protect };
        if (m.Type != 0x20000) {
          string o; long ab = b.AyirmaTabani;
          if (!ad.TryGetValue(ab, out o)) {
            var sb = new StringBuilder(520);
            o = GetMappedFileNameW(h, (IntPtr)taban, sb, 520) > 0 ? sb.ToString() : "(adsiz eşleme)";
            ad[ab] = o;
          }
          b.Ad = o;
        }
        // çalışma kümesi: bu bölgedeki sayfalar, kaçı paylaşılan?
        long sayfa = boy / 4096; long ofs = 0;
        while (ofs < sayfa) {
          int n = (int)Math.Min(4096, sayfa - ofs);
          var ws = new WSEX[n];
          for (int i = 0; i < n; i++) ws[i].Adres = (IntPtr)(taban + (ofs + i) * 4096);
          if (QueryWorkingSetEx(h, ws, n * Marshal.SizeOf(typeof(WSEX)))) {
            for (int i = 0; i < n; i++) {
              ulong f = (ulong)ws[i].Bayrak;
              if ((f & 1) != 0) { b.Sayfa++; if ((f & (1UL << 15)) != 0) b.Paylasilan++; }
            }
          }
          ofs += n;
        }
        liste.Add(b);
      }
      adr = taban + boy;
    }
    return liste;
  }

  // Bölgeden örnek: ilk 32 bayt (hex), 64 sayfa örneğinde sıfır oranı.
  public static string Ornek(IntPtr h, long taban, long boy) {
    var b = new byte[32]; IntPtr rd; string hex = "(okunamadı)";
    if (ReadProcessMemory(h, (IntPtr)taban, b, 32, out rd)) hex = BitConverter.ToString(b, 0, 16).Replace("-", "");
    long sayfa = boy / 4096; if (sayfa < 1) sayfa = 1; long adim = Math.Max(1, sayfa / 64); int oku = 0, sifir = 0, okunamayan = 0;
    var pg = new byte[256];
    for (long i = 0; i < sayfa; i += adim) {
      if (ReadProcessMemory(h, (IntPtr)(taban + i * 4096), pg, 256, out rd)) {
        oku++; bool z = true; for (int j = 0; j < 256; j++) if (pg[j] != 0) { z = false; break; } if (z) sifir++;
      } else okunamayan++;
    }
    return hex + " sıfır=" + (oku == 0 ? 0 : sifir * 100 / oku) + "% okunan=" + oku;
  }

  // Bölgenin içine bakış: birkaç ofsette 32 bayt hex + okunabilir ASCII/UTF-16 metin parçaları.
  public static string Bak(IntPtr h, long taban, long boy) {
    var sb = new StringBuilder(); IntPtr rd;
    long[] ofs = { 0, 0x1000, 0xE000, 0x10000, 0x40000, boy / 2, boy - 64 };
    foreach (long o in ofs) {
      if (o < 0 || o + 32 > boy) continue;
      var b = new byte[32];
      if (ReadProcessMemory(h, (IntPtr)(taban + o), b, 32, out rd)) sb.AppendLine("      +0x" + o.ToString("X") + ": " + BitConverter.ToString(b).Replace("-", ""));
    }
    // metin örneği: ilk 4 MB içinde 6+ karakterlik ASCII dizileri (en çok 6 tanesi)
    long tara = Math.Min(boy, 4L << 20); var buf = new byte[tara]; int n = 0; var sb2 = new StringBuilder();
    if (ReadProcessMemory(h, (IntPtr)taban, buf, (int)tara, out rd)) {
      int basla = -1;
      for (int i = 0; i <= tara && n < 6; i++) {
        bool ok = i < tara && buf[i] >= 32 && buf[i] < 127;
        if (ok && basla < 0) basla = i;
        if (!ok && basla >= 0) { if (i - basla >= 8) { sb2.Append("\"" + Encoding.ASCII.GetString(buf, basla, Math.Min(60, i - basla)) + "\" "); n++; } basla = -1; }
      }
    }
    sb.AppendLine("      metin: " + (n == 0 ? "(yok)" : sb2.ToString()));
    return sb.ToString().TrimEnd();
  }

  // Bölgenin ham içeriğini dosyaya yazar (inceleme için).
  public static void Kaydet(IntPtr h, long taban, long boy, string dosya) {
    IntPtr rd; var buf = new byte[boy];
    if (ReadProcessMemory(h, (IntPtr)taban, buf, (int)boy, out rd)) System.IO.File.WriteAllBytes(dosya, buf);
  }

  // Özel ayırmanın kimliği, ilk işlenmiş bölgesinin başındaki başlıktan:
  //   "heap-kesim": NT heap kesimi (HEAP_SEGMENT, +0x10'da 0xFFEEFFEE imzası; +0x28 sahibi heap)
  //   "heap-buyuk": heap'in VirtualAlloc ile ayırdığı büyük blok (HEAP_VIRTUAL_ALLOC_ENTRY:
  //                 +0x00 Flink, +0x08 Blink, +0x20 CommitSize, +0x28 ReserveSize)
  // Büyük bloklar (x64'te ~1 MB üstü) heap'in BytesCommitted sayısına girmeyebilir; ayrı sayılır.
  public static string Sinif(IntPtr h, long taban, long islenen, out long heap) {
    heap = 0; var b = new byte[0x40]; IntPtr rd;
    if (!ReadProcessMemory(h, (IntPtr)taban, b, 0x40, out rd)) return "okunamadı";
    if (BitConverter.ToUInt32(b, 0x10) == 0xFFEEFFEE) { heap = BitConverter.ToInt64(b, 0x28); return "heap-kesim"; }
    long fl = BitConverter.ToInt64(b, 0), bl = BitConverter.ToInt64(b, 8);
    long cs = BitConverter.ToInt64(b, 0x20), rs = BitConverter.ToInt64(b, 0x28);
    bool ptr = fl > 0x10000 && bl > 0x10000 && fl < 0x7FFFFFFFFFFFL && bl < 0x7FFFFFFFFFFFL;
    if (ptr && cs > 0 && rs >= cs && Math.Abs(cs - islenen) <= 0x10000) return "heap-buyuk";
    return "";
  }

  // İşaretçiyi tutan nesnenin sahibi için ipucu: adresten geriye 0x200 bayt
  // içinde bir modülün (exe/dll) içine işaret eden en yakın değer (C++ vtable'ı,
  // Rust'ta dyn vtable'ı ya da modülün global verisi). "modül+bölge koruması@uzaklık".
  public static string Komsu(IntPtr h, List<Bolge> bolgeler, long adr) {
    var b = new byte[0x208]; IntPtr rd; long bas = adr - 0x200;
    if (!ReadProcessMemory(h, (IntPtr)bas, b, b.Length, out rd)) return "?";
    for (int i = 0x200; i >= 0; i -= 8) {
      long v = BitConverter.ToInt64(b, i);
      if (v < 0x10000) continue;
      foreach (var g in bolgeler) {
        if (g.Tur != 0x1000000 || v < g.Taban || v >= g.Taban + g.Boyut) continue;
        string ad = g.Ad ?? "?"; int k = ad.LastIndexOf('\\'); if (k >= 0) ad = ad.Substring(k + 1);
        return ad + ":" + g.Koruma.ToString("X") + "@-" + (0x200 - i).ToString("X");
      }
    }
    return "-";
  }

  // Kim tutuyor? Okunabilir bütün bölgelerde (yazma-birleştirmeli hariç) 8 bayt
  // hizalı değerleri tarar; [bas[i], bas[i]+0x1000) aralığına düşen her değer
  // için "hedef<TAB>bulunduğu bölgenin ayırma tabanı<TAB>adı" döndürür.
  public static List<string> Referans(IntPtr h, List<Bolge> bolgeler, long[] bas) {
    var r = new List<string>(); if (bas.Length == 0) return r;
    long lo = long.MaxValue, hi = 0; foreach (var x in bas) { lo = Math.Min(lo, x); hi = Math.Max(hi, x + 0x1000); }
    var buf = new byte[1 << 20]; IntPtr rd;
    foreach (var b in bolgeler) {
      uint k = b.Koruma;
      if ((k & 0x101) != 0 || (k & 0xEE) == 0) continue;           // erişilemez / bekçi
      if (b.Tur == 0x20000 && (k & 0x400) != 0) continue;           // yazma-birleştirmeli: yavaş
      for (long ofs = 0; ofs < b.Boyut; ofs += buf.Length) {
        int n = (int)Math.Min(buf.Length, b.Boyut - ofs);
        if (!ReadProcessMemory(h, (IntPtr)(b.Taban + ofs), buf, n, out rd)) continue;
        for (int i = 0; i + 8 <= n; i += 8) {
          long v = BitConverter.ToInt64(buf, i);
          if (v < lo || v >= hi) continue;
          foreach (var x in bas) if (v >= x && v < x + 0x1000) {
            long adr = b.Taban + ofs + i;
            if (adr >= x - 0x40 && adr < x + 0x1000) continue;     // kendi içi
            r.Add(x.ToString("X") + "\t" + b.AyirmaTabani.ToString("X") + "\t" + (b.Ad ?? "") + "\t" + b.Tur.ToString("X") + "\t" + Komsu(h, bolgeler, adr));
          }
        }
      }
    }
    return r;
  }

  // Ayırma tabanındaki ilk 0x40 baytı 8'er baytlık sayılar olarak (hex) verir.
  public static string Baslik(IntPtr h, long taban) {
    var b = new byte[0x40]; IntPtr rd;
    if (!ReadProcessMemory(h, (IntPtr)taban, b, 0x40, out rd)) return "(okunamadı)";
    var sb = new StringBuilder();
    for (int i = 0; i < 0x40; i += 8) sb.Append(BitConverter.ToUInt64(b, i).ToString("X") + " ");
    return sb.ToString().TrimEnd();
  }

  public static long AyirmaTabani(IntPtr h, long adr) {
    MBI m; if (VirtualQueryEx(h, (IntPtr)adr, out m, Marshal.SizeOf(typeof(MBI))) == 0) return 0; return (long)m.AllocationBase;
  }

  // Her iş parçacığının yığıtı: TEB.StackBase (0x08) / StackLimit (0x10).
  public static List<Yigit> Yigitlar(IntPtr h, ProcessThreadCollection th) {
    var r = new List<Yigit>();
    foreach (ProcessThread t in th) {
      IntPtr ht = OpenThread(0x0040 | 0x0800, false, (uint)t.Id);
      if (ht == IntPtr.Zero) continue;
      var y = new Yigit { Tid = t.Id, Ad = "" };
      IntPtr np; if (GetThreadDescription(ht, out np) >= 0 && np != IntPtr.Zero) { y.Ad = Marshal.PtrToStringUni(np); LocalFree(np); }
      var buf = new byte[48]; int ret;
      if (NtQueryInformationThread(ht, 0, buf, 48, out ret) == 0) {
        long teb = BitConverter.ToInt64(buf, 8);
        var tb = new byte[0x20]; IntPtr rd;
        if (ReadProcessMemory(h, (IntPtr)teb, tb, 0x20, out rd)) {
          y.Taban = BitConverter.ToInt64(tb, 8); y.Limit = BitConverter.ToInt64(tb, 16);
          y.AyirmaTabani = AyirmaTabani(h, y.Taban - 1);
        }
      }
      CloseHandle(ht); r.Add(y);
    }
    return r;
  }

  // NT heap'leri (Rtl* yan yolu). Başarısız olursa null.
  public static List<HeapBilgi> Heapler(int pid, out string hata) {
    hata = null; var r = new List<HeapBilgi>();
    IntPtr buf = RtlCreateQueryDebugBuffer(0x1000000, false);
    if (buf == IntPtr.Zero) { hata = "RtlCreateQueryDebugBuffer başarısız"; return null; }
    int st = RtlQueryProcessDebugInformation((IntPtr)pid, 0x04, buf);   // PDI_HEAPS
    if (st != 0) { hata = "RtlQueryProcessDebugInformation 0x" + st.ToString("X"); RtlDestroyQueryDebugBuffer(buf); return null; }
    long heaps = Marshal.ReadInt64(buf, 112);          // RTL_DEBUG_INFORMATION.Heaps (x64)
    if (heaps == 0) { hata = "Heaps işaretçisi boş"; RtlDestroyQueryDebugBuffer(buf); return null; }
    uint n = (uint)Marshal.ReadInt32((IntPtr)heaps);
    if (Environment.GetEnvironmentVariable("DOKUM_HAM") != null) {
      var sb = new StringBuilder("ham n=" + n + Environment.NewLine);
      for (int j = 0; j < 64; j++) sb.Append(Marshal.ReadInt64((IntPtr)(heaps + j * 8)).ToString("X") + ((j % 4 == 3) ? Environment.NewLine : " "));
      hata = sb.ToString();
    }
    for (uint i = 0; i < n && i < 512; i++) {
      long e = heaps + 8 + i * 96;                     // RTL_HEAP_INFORMATION (96 bayt)
      r.Add(new HeapBilgi {
        Taban = Marshal.ReadInt64((IntPtr)e), Bayrak = (uint)Marshal.ReadInt32((IntPtr)(e + 8)),
        Ayrilan = Marshal.ReadInt64((IntPtr)(e + 16)), Islenen = Marshal.ReadInt64((IntPtr)(e + 24)) });
    }
    RtlDestroyQueryDebugBuffer(buf);
    return r;
  }
}
"@
}
[Dokum]::SetProcessDPIAware() | Out-Null

function MB($b) { [math]::Round($b / 1MB, 1) }

# bellek-olcumu.ps1 ile aynı tekerlek: pencereye WM_MOUSEWHEEL gönderir (gerçek fare oynamaz).
function Wheel($h, $r, $x, $dir, $n, $ms) {
    $sx = $r.L + 8 + $x; $sy = $r.T + 1 + 400
    $lp = [IntPtr]((($sy -band 0xFFFF) -shl 16) -bor ($sx -band 0xFFFF))
    $wp = [IntPtr][long]((((120 * $dir) -band 0xFFFF)) -shl 16)
    for ($i = 0; $i -lt $n; $i++) {
        [Dokum]::PostMessage($h, 0x020A, $wp, $lp) | Out-Null
        Start-Sleep -Milliseconds $ms
    }
}

function DokumYap($p) {
    $h = [Dokum]::Ac($p.Id)
    $regions = [Dokum]::Gez($h)
    $stacks = [Dokum]::Yigitlar($h, $p.Threads)
    $hata = $null
    $heaps = [Dokum]::Heapler($p.Id, [ref]$hata)
    if ($env:DOKUM_HAM) { $hata }
    $p.Refresh()
    "=== DÖKÜM  [$Label]  özel=$(MB $p.PrivateMemorySize64) MB  çalışma=$(MB $p.WorkingSet64) MB  iş parçacığı=$($p.Threads.Count)  tutamaç=$($p.HandleCount)"

    # 1) tür bazında işlenmiş bellek ve çalışma kümesi
    $turAd = @{ 0x20000 = 'ÖZEL (private)'; 0x40000 = 'EŞLENMİŞ (mapped)'; 0x1000000 = 'GÖRÜNTÜ (image: exe/dll)' }
    "--- Türe göre (işlenmiş MB | çalışma kümesinde MB | bunun paylaşılan MB)"
    $regions | Group-Object Tur | ForEach-Object {
        $c = ($_.Group | Measure-Object Boyut -Sum).Sum
        $w = ($_.Group | Measure-Object Sayfa -Sum).Sum * 4096
        $s = ($_.Group | Measure-Object Paylasilan -Sum).Sum * 4096
        "{0,-26} işlenmiş {1,7} | çalışma {2,7} | paylaşılan {3,7}" -f $turAd[[int]$_.Name], (MB $c), (MB $w), (MB $s)
    }

    # 2) özel belleğin dağılımı: heap / yığıt / geri kalan
    $priv = @($regions | Where-Object { $_.Tur -eq 0x20000 })
    $privTotal = ($priv | Measure-Object Boyut -Sum).Sum
    $stackBases = @{}
    $stackCommit = 0
    foreach ($s in $stacks) { if ($s.AyirmaTabani -and -not $stackBases.ContainsKey($s.AyirmaTabani)) { $stackBases[$s.AyirmaTabani] = $s } }
    foreach ($b in $stackBases.Keys) {
        $c = ($priv | Where-Object { $_.AyirmaTabani -eq $b } | Measure-Object Boyut -Sum).Sum
        $stackBases[$b].Islenen = $c; $stackCommit += $c
    }
    $heapAlloc = 0; $heapCommit = 0
    if ($heaps) {
        $heapAlloc = ($heaps | Measure-Object Ayrilan -Sum).Sum
        $heapCommit = ($heaps | Measure-Object Islenen -Sum).Sum
    }
    $heapBasesAlloc = @{}
    if ($heaps) { foreach ($hp in $heaps) { $hp.AyirmaTabani = [Dokum]::AyirmaTabani($h, $hp.Taban); $heapBasesAlloc[$hp.AyirmaTabani] = 1 } }
    # Her özel ayırmayı (ayırma tabanına göre) tek bir sınıfa koy; sınıflar
    # toplamı özel toplama eşit olur (çift sayım yok):
    #   yığıt > heap-kesim > heap-buyuk > yazma-birleştirmeli (sürücü/GPU) > diğer
    $sinif = @{}
    $allocs = foreach ($g in ($priv | Group-Object AyirmaTabani)) {
        $ab = [long]$g.Name
        $c = ($g.Group | Measure-Object Boyut -Sum).Sum
        $w = ($g.Group | Measure-Object Sayfa -Sum).Sum * 4096
        $wc = @($g.Group | Where-Object { ($_.Koruma -band 0x400) -ne 0 }).Count -gt 0
        $owner = [long]0
        $k = if ($stackBases.ContainsKey($ab)) { 'yigit' } else {
            # Başlık ilk işlenmiş bölgenin başında: büyük bloklarda Windows
            # başlangıcı ayırma içinde rastgele kaydırıyor (+0x2000, +0x9000 ...).
            $s = [Dokum]::Sinif($h, $g.Group[0].Taban, $c, [ref]$owner)
            if ($s -eq 'heap-kesim') { 'heapKesim' } elseif ($s -eq 'heap-buyuk') { 'heapBuyuk' } elseif ($wc) { 'gpuWC' } else { 'diger' }
        }
        $sinif[$ab] = $k
        [pscustomobject]@{ Taban = $ab; Sinif = $k; C = $c; W = $w; Sahip = $owner }
    }
    $sinifAd = [ordered]@{
        heapKesim = 'NT heap kesimleri (Rust + C/DLL ayırmaları)'
        heapBuyuk = "NT heap büyük blokları (~1 MB üstü tek ayırma)"
        gpuWC     = 'yazma-birleştirmeli (sürücü / GPU, PAGE_WRITECOMBINE)'
        yigit     = 'iş parçacığı yığıtları'
        diger     = 'diğer VirtualAlloc'
    }
    $script:sinifTop = @{}
    "--- Özel (private) bellek sınıfları: toplam $(MB $privTotal) MB işlenmiş (sınıfların toplamı = bu)"
    foreach ($k in $sinifAd.Keys) {
        $set = @($allocs | Where-Object { $_.Sinif -eq $k })
        $c = ($set | Measure-Object C -Sum).Sum; $w = ($set | Measure-Object W -Sum).Sum
        $script:sinifTop[$k] = @{ C = $c; W = $w }
        "  {0,-55} işlenmiş {1,6} MB | çalışmada {2,6} MB | {3,3} ayırma" -f $sinifAd[$k], (MB $c), (MB $w), $set.Count
    }
    if ($heaps) {
        "  (çapraz denetim) heap API'si: $(MB $heapCommit) MB işlenmiş, $(MB $heapAlloc) MB ayrılmış ($($heaps.Count) heap); bölge taramasıyla kesimler $(MB $script:sinifTop.heapKesim.C) MB"
        foreach ($hp in ($heaps | Sort-Object Islenen -Descending)) {
            $kes = ($allocs | Where-Object { $_.Sinif -eq 'heapKesim' -and $_.Sahip -eq $hp.Taban } | Measure-Object C -Sum).Sum
            "    heap 0x{0:X} bayrak=0x{1:X}: API işlenen {2,6} MB, ayrılan {3,6} MB | taramada kesimleri {4,6} MB" -f $hp.Taban, $hp.Bayrak, (MB $hp.Islenen), (MB $hp.Ayrilan), (MB $kes)
        }
    } else {
        "  NT heap bilgisi alınamadı: $hata"
    }
    "--- Yığıtlar (işlenen KB), ada göre"
    $stackBases.Values | Group-Object { if ($_.Ad) { $_.Ad } else { '(adsız)' } } | ForEach-Object {
        $sum = ($_.Group | Measure-Object Islenen -Sum).Sum
        "  {0,-28} x{1,-2} {2,7} KB" -f $_.Name, $_.Count, [math]::Round($sum / 1KB)
    }

    # 3) en büyük özel bölgeler (ayırma tabanına göre toplanmış)
    "--- En büyük özel ayırmalar (taban, işlenen MB, işaret)"
    $priv | Group-Object AyirmaTabani | ForEach-Object {
        [pscustomobject]@{ Taban = [long]$_.Name; Islenen = ($_.Group | Measure-Object Boyut -Sum).Sum; N = $_.Count }
    } | Sort-Object Islenen -Descending | Select-Object -First 14 | ForEach-Object {
        $tag = if ($stackBases.ContainsKey($_.Taban)) { 'yığıt: ' + $stackBases[$_.Taban].Ad } elseif ($heapBasesAlloc.ContainsKey($_.Taban)) { 'NT heap tabanı' } else { $sinif[$_.Taban] }
        "  0x{0:X12} {1,7} MB  ({2} parça) {3}" -f $_.Taban, (MB $_.Islenen), $_.N, $tag
    }
    "--- Ayrıntı: en büyük 12 özel ayırma (işlenen MB, çalışma kümesinde MB, koruma, ilk 16 bayt, sıfır sayfa oranı)"
    $priv | Group-Object AyirmaTabani | ForEach-Object {
        $g = $_.Group
        [pscustomobject]@{ Taban = [long]$_.Name; C = ($g | Measure-Object Boyut -Sum).Sum; W = ($g | Measure-Object Sayfa -Sum).Sum * 4096; Kor = ($g | ForEach-Object { '{0:X}' -f $_.Koruma } | Select-Object -Unique) -join '/'; Ilk = $g[0] }
    } | Sort-Object C -Descending | Select-Object -First 12 | ForEach-Object {
        $o = [Dokum]::Ornek($h, $_.Ilk.Taban, $_.Ilk.Boyut)
        "  0x{0:X12} işlenen {1,6} MB | çalışmada {2,6} MB | koruma {3} | {4} | {5}" -f $_.Taban, (MB $_.C), (MB $_.W), $_.Kor, $o, $sinif[$_.Taban]
        if ($sinif[$_.Taban] -eq 'diger') { "      ilk işlenmiş bölge +0x{0:X}, boyu {1} KB; başlık (8 baytlık): {2}" -f ($_.Ilk.Taban - $_.Taban), ($_.Ilk.Boyut / 1KB), [Dokum]::Baslik($h, $_.Ilk.Taban) }
        if ($Bak) {
            [Dokum]::Bak($h, $_.Ilk.Taban, $_.Ilk.Boyut)
            if ($env:DOKUM_KAYDET -and $_.C -ge 2MB -and $_.Ilk.Boyut -eq $_.C) { [Dokum]::Kaydet($h, $_.Ilk.Taban, $_.Ilk.Boyut, (Join-Path $env:DOKUM_KAYDET ('{0:X}.bin' -f $_.Taban))) }
        }
    }
    if ($Referans) {
        "--- Kim tutuyor: heap büyük blokları, yazma-birleştirmeli ve 1 MB üstü diğer ayırmalar için, başına işaret eden 8 baytlık değerlerin yeri"
        $hedef = @($allocs | Where-Object { ($_.Sinif -in 'heapBuyuk', 'gpuWC') -or ($_.Sinif -eq 'diger' -and $_.C -ge 1MB) })
        $basMap = @{}
        foreach ($a in $hedef) {
            $ilk = ($priv | Where-Object { $_.AyirmaTabani -eq $a.Taban } | Select-Object -First 1).Taban
            $kul = if ($a.Sinif -eq 'heapBuyuk') { $ilk + 0x40 } else { $ilk }
            $basMap[('{0:X}' -f $kul)] = $a
        }
        $bas = [long[]]@($basMap.Keys | ForEach-Object { [Convert]::ToInt64($_, 16) })
        $refs = [Dokum]::Referans($h, $regions, $bas)
        foreach ($key in ($basMap.Keys | Sort-Object { $basMap[$_].C } -Descending)) {
            $a = $basMap[$key]
            $mine = @($refs | Where-Object { $_.StartsWith($key + "`t") })
            $ozet = $mine | ForEach-Object {
                $f = $_ -split "`t"
                if ($f[3] -eq '20000') { $s2 = $sinif[[Convert]::ToInt64($f[1], 16)]; "özel:$s2 (yakın: $($f[4]))" } else { $n = $f[2]; ($n -split '\\')[-1] }
            } | Group-Object | Sort-Object Count -Descending | ForEach-Object { "$($_.Name) x$($_.Count)" }
            "  0x{0} {1,6} MB {2,-9} <- {3}" -f $key, (MB $a.C), $a.Sinif, ($(if ($ozet) { $ozet -join ', ' } else { '(işaretçi bulunamadı)' }))
        }
    }
    "--- Özel ayırma boyut dağılımı: sayı ve toplam"
    $buckets = @(
        @{ n = '< 64 KB'; lo = 0; hi = 64KB }, @{ n = '64 KB - 1 MB'; lo = 64KB; hi = 1MB },
        @{ n = '1 - 4 MB'; lo = 1MB; hi = 4MB }, @{ n = '>= 4 MB'; lo = 4MB; hi = 1GB })
    $alloc = $priv | Group-Object AyirmaTabani | ForEach-Object { ($_.Group | Measure-Object Boyut -Sum).Sum }
    foreach ($bk in $buckets) {
        $set = @($alloc | Where-Object { $_ -ge $bk.lo -and $_ -lt $bk.hi })
        "  {0,-14} {1,5} ayırma  {2,7} MB" -f $bk.n, $set.Count, (MB (($set | Measure-Object -Sum).Sum))
    }

    # 4) eşlenmiş ve görüntü: dosya bazında (çalışma kümesi MB)
    "--- Eşlenmiş + görüntü bölgeleri, dosya bazında (işlenmiş MB | çalışma MB | özel-benzeri = paylaşılmayan çalışma MB)"
    $regions | Where-Object { $_.Tur -ne 0x20000 } | Group-Object Ad | ForEach-Object {
        $c = ($_.Group | Measure-Object Boyut -Sum).Sum
        $w = ($_.Group | Measure-Object Sayfa -Sum).Sum * 4096
        $s = ($_.Group | Measure-Object Paylasilan -Sum).Sum * 4096
        [pscustomobject]@{ Ad = $_.Name; C = $c; W = $w; Pr = $w - $s }
    } | Sort-Object W -Descending | Select-Object -First 16 | ForEach-Object {
        $ad = $_.Ad; if ($ad.Length -gt 62) { $ad = '...' + $ad.Substring($ad.Length - 59) }
        "  {0,-62} {1,6} | {2,6} | {3,6}" -f $ad, (MB $_.C), (MB $_.W), (MB $_.Pr)
    }
    "--- Yüklü modüller (toplam $($p.Modules.Count)); en büyük 14 (MB, adres uzayı)"
    $p.Modules | Sort-Object ModuleMemorySize -Descending | Select-Object -First 14 | ForEach-Object { "  {0,-34} {1,6}" -f $_.ModuleName, (MB $_.ModuleMemorySize) }
    if ($env:DOKUM_MODUL) { $p.Modules | ForEach-Object { 'MODUL ' + $_.ModuleName + ' ' + $_.ModuleMemorySize } | Out-File $env:DOKUM_MODUL -Encoding utf8 }
    # 5) yazılabilir görüntü bölgeleri: "özel bayt" (commit charge) bunları da sayar
    $wr = @($regions | Where-Object { $_.Tur -eq 0x1000000 -and (($_.Koruma -band 0xCC) -ne 0) })
    $wrTotal = ($wr | Measure-Object Boyut -Sum).Sum
    "--- Yazılabilir (WRITE/WRITECOPY) görüntü bölgeleri: toplam $(MB $wrTotal) MB işlenmiş  (commit charge'a girer, ama çoğu dokunulmamış)"
    $wr | Group-Object Ad | ForEach-Object {
        [pscustomobject]@{ Ad = $_.Name; C = ($_.Group | Measure-Object Boyut -Sum).Sum; W = ($_.Group | Measure-Object Sayfa -Sum).Sum * 4096 }
    } | Sort-Object C -Descending | Select-Object -First 6 | ForEach-Object {
        $ad = $_.Ad; if ($ad.Length -gt 62) { $ad = '...' + $ad.Substring($ad.Length - 59) }
        "  {0,-62} işlenmiş {1,6} | çalışmada {2,6}" -f $ad, (MB $_.C), (MB $_.W)
    }
    $wcR = @($priv | Where-Object { ($_.Koruma -band 0x400) -ne 0 })
    $wcC = ($wcR | Measure-Object Boyut -Sum).Sum; $wcW = ($wcR | Measure-Object Sayfa -Sum).Sum * 4096
    "--- Yazma-birleştirmeli (PAGE_WRITECOMBINE 0x400) özel bellek = sürücü/GPU yükleme tamponları: işlenmiş $(MB $wcC) MB, çalışma kümesinde $(MB $wcW) MB"
    $mwc = @($regions | Where-Object { $_.Tur -eq 0x40000 -and (($_.Koruma -band 0x88) -ne 0) })
    $mwcTotal = ($mwc | Measure-Object Boyut -Sum).Sum
    "--- Yazarken-kopyala (WRITECOPY) eşlenmiş bölgeler: $(MB $mwcTotal) MB"
    $p.Refresh()
    $sayac = $p.PrivateMemorySize64
    $script:ozet = [ordered]@{
        sayac = (MB $sayac); ozelBolge = (MB $privTotal); yazgor = (MB $wrTotal)
        heapKesim = (MB $script:sinifTop.heapKesim.C); heapBuyuk = (MB $script:sinifTop.heapBuyuk.C)
        gpuWC = (MB $script:sinifTop.gpuWC.C); yigit = (MB $script:sinifTop.yigit.C); diger = (MB $script:sinifTop.diger.C)
        heapKesimW = (MB $script:sinifTop.heapKesim.W); heapBuyukW = (MB $script:sinifTop.heapBuyuk.W)
        gpuWCW = (MB $script:sinifTop.gpuWC.W); digerW = (MB $script:sinifTop.diger.W)
        heapApi = (MB $heapCommit)
    }
    "--- Özet: özel bölgeler $(MB $privTotal) + yazılabilir görüntü $(MB $wrTotal) = $(MB ($privTotal + $wrTotal)) MB  | süreç sayacı (PrivateMemorySize64, döküm sonunda) $(MB $sayac) MB  | fark $(MB ($sayac - $privTotal - $wrTotal)) MB"
    [Dokum]::Kapat($h)
}

$allRows = @()
for ($rep = 1; $rep -le $Repeat; $rep++) {
    $binDir = Join-Path $env:TEMP ("openify-bellek\" + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Force $binDir | Out-Null
    $runExe = Join-Path $binDir 'openify.exe'
    Copy-Item $srcExe $runExe
    # Sembol dosyası da yanına (geri izlemelerde işlev adları için).
    $pdb = [System.IO.Path]::ChangeExtension($srcExe, '.pdb')
    if (Test-Path $pdb) { Copy-Item $pdb $binDir }
    $savedEnv = @{}
    $rows = @()
    $logStart = $null
    $p = $null
    try {
        [IO.File]::WriteAllText($settings, '{ "volume": 0.0 }', (New-Object System.Text.UTF8Encoding($false)))
        $env:OPENIFY_SETTINGS_PATH = $settings
        $fg = [Dokum]::GetForegroundWindow()
        $env:OPENIFY_TEST_DISPLAY = "1180,760"
        $env:OPENIFY_TEST_FAKE_PLAY = "1"
        foreach ($k in $extraEnv.Keys) {
            $savedEnv[$k] = [Environment]::GetEnvironmentVariable($k)
            [Environment]::SetEnvironmentVariable($k, $extraEnv[$k])
        }
        $logPath = "$env:LOCALAPPDATA\Openify\openify.log"
        $logStart = (Get-Item $logPath -ErrorAction SilentlyContinue).Length
        $p = Start-Process $runExe -PassThru
        $procStart = $p.StartTime.ToUniversalTime()
        $restored = $false; $restoreAt = 0
        $pending = [System.Collections.Generic.Queue[double]]::new()
        foreach ($t in ($atList | Sort-Object)) { $pending.Enqueue($t) }
                while ($pending.Count -gt 0) {
            $now = ([datetime]::UtcNow - $procStart).TotalSeconds
            if (-not $restored) {
                $p.Refresh()
                if ($p.MainWindowHandle -ne 0) {
                    if ($restoreAt -eq 0) { $restoreAt = $now + 0.3 }
                    elseif ($now -ge $restoreAt) { [Dokum]::Restore($fg) | Out-Null; $restored = $true }
                }
            }
            if ($now -ge $pending.Peek()) {
                $t = $pending.Dequeue()
                $p.Refresh()
                $rows += [pscustomobject]@{ etiket = $Label; deneme = $rep; t = $t; ozelMB = (MB $p.PrivateMemorySize64); calismaMB = (MB $p.WorkingSet64); sayfaliMB = (MB $p.PagedMemorySize64) }
                if ($pending.Count -eq 0 -and $Detay -and -not $Kaydir) { DokumYap $p }
            } else { Start-Sleep -Milliseconds 20 }
        }
        if ($Kaydir) {
            $p.Refresh()
            if (-not $restored) { [Dokum]::Restore($fg) | Out-Null; $restored = $true }
            $hw = $p.MainWindowHandle
            if ($hw -eq 0) { throw "pencere bulunamadı, kaydırılamadı" }
            $r = New-Object Dokum+RECT; [Dokum]::GetWindowRect($hw, [ref]$r) | Out-Null
            for ($round = 1; $round -le 4; $round++) {
                Wheel $hw $r 700 -1 40 50      # liste aşağı
                Start-Sleep -Milliseconds 800
                Wheel $hw $r 140 -1 20 50      # kenar çubuğu aşağı
                Start-Sleep -Milliseconds 800
                Wheel $hw $r 140 1 20 50       # kenar çubuğu yukarı
                Start-Sleep -Milliseconds 1500
            }
            Start-Sleep -Seconds 4
            $p.Refresh()
            $rows += [pscustomobject]@{ etiket = $Label; deneme = $rep; t = 'kaydir'; ozelMB = (MB $p.PrivateMemorySize64); calismaMB = (MB $p.WorkingSet64); sayfaliMB = (MB $p.PagedMemorySize64) }
            if ($Detay) { DokumYap $p }
        }
    } finally {
        # Hata olsa da kopya exe açık kalmasın.
        if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -ErrorAction SilentlyContinue; $p.WaitForExit(5000) | Out-Null }
        foreach ($k in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($k, $savedEnv[$k]) }
        Remove-Item Env:OPENIFY_TEST_FAKE_PLAY -ErrorAction SilentlyContinue
        Remove-Item Env:OPENIFY_TEST_DISPLAY -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500
        Remove-Item Env:OPENIFY_SETTINGS_PATH -ErrorAction SilentlyContinue
        Remove-Item $settings -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $binDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    "--- [$Label] deneme $rep/$Repeat  (özel MB / çalışma kümesi MB; t = işlemin gerçek başlangıcından beri sn)"
    $rows | Format-Table etiket, deneme, t, ozelMB, calismaMB, sayfaliMB -AutoSize | Out-String | Write-Host
    if ($Csv) {
        $rows | ForEach-Object { "$($_.etiket),$($_.deneme),$($_.t),$($_.ozelMB),$($_.calismaMB)" } | Add-Content $Csv
        if ($Detay -and $script:ozet) {
            foreach ($k in $script:ozet.Keys) { "$Label,$rep,$k,$($script:ozet[$k]),0" | Add-Content $Csv }
        }
    }
    if ($Gunluk -and $logStart -ne $null) {
        $fs = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
        try {
            $fs.Seek($logStart, 'Begin') | Out-Null
            $text = (New-Object System.IO.StreamReader($fs)).ReadToEnd()
            "--- günlük (aşama satırları)"
            $t0 = $procStart
            foreach ($ln in ($text -split "`n")) {
                if ($ln -notmatch 'openify::') { continue }
                $mt = [regex]::Match($ln, '^\[(\S+?)Z')
                $rel = ''
                if ($mt.Success) { $rel = '{0,6} ms' -f [int]([datetime]::Parse($mt.Groups[1].Value, [System.Globalization.CultureInfo]::InvariantCulture, [System.Globalization.DateTimeStyles]::AssumeUniversal).ToUniversalTime() - $t0).TotalMilliseconds }
                $short = ($ln -replace '^\[\S+\s+', '').Trim()
                "  $rel  $short"
            }
        } finally { $fs.Dispose() }
    }
    $allRows += $rows
}
