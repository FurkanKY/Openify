#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Openify gerçek çalma oturumu (scripts/ses-oturumu.ps1) çözümleyicisi.

Kullanım:
    python -I scripts/ses_oturumu_ozet.py <iz.csv> <openify.log ya da dilim> [--marks <faz.csv>] [eşikler]

<iz.csv>: OPENIFY_SINK_TRACE çıktısı
    t_unix_ms,event,queued_ms,xruns,gain,target_ms,underrun_ms
<log>: openify.log'un tamamı ya da oturumun dilimi (ses-oturumu.ps1 dilimi yazar).
--marks: ses-oturumu.ps1'in yazdığı faz dosyası (phase,start_unix_ms,end_unix_ms);
    verilmezse <iz>.csv yanındaki <iz>.marks.csv aranır; yoksa "soak" penceresi
    bütün iz olur (not düşülür).

Kriterler (hepsi eşik parametresi, varsayılanlar parantezde):
  duraklat -> sessiz (iç)        <= 20 ms ; + D (30) < 60 ms
  ses düzeyi -> gain (iç)        <= 11 ms ; + D < 40 ms
  sarma -> sessiz                <= 20 ms ; sarma -> duyulur + D < 150 ms
  soak: xrun = 0, underrun_ms = 0, target_ms sabit (120)
  boşluksuz geçiş: auto_next çevresinde xrun yok, queued_ms > 0
  duraklatılmış konum: "Devam duyulan" == önceki "Duraklatıldı duyulan" (+-2 ms)
  ek: oturum boyunca xrun = 0; logda "Ses çıkışı boşaldı" = 0
Bilgi (ölçüt değil): her auto_next için yeni şarkının ilk paketi (first_write)
gecikmesi ve o an kuyrukta kalan ses; producer_gap satırları (çalarken çözücünün
iki paket arası > 200 ms); her xrun ve 1 sn'yi aşan üretici boşluğu için Windows
Sistem günlüğünde +-10 sn içindeki NTFS (130/132/133) / chkdsk / volsnap /
Kernel-Power / disk olayları (tek PowerShell çağrısı; --no-events kapatır):
disk kaynaklı kesinti "DİSK OLAYI" diye etiketlenir.

İzdeki satırlar zamana göre sıralı değildir (ses iş parçacığının anları bir tik
sonra yazılır): önce zamana göre sıralanır. Sayısal sütunlar (kuyruk, xrun,
underrun) yalnız `tick` satırlarında o anın değeridir (anı olan satırlarda yazım
anının değeri); pencere hesapları bu yüzden tik satırlarıyla yapılır.

Çıkış kodu: hepsi PASS ise 0, herhangi biri FAIL ise 1, girdi okunamazsa 2.
"""
import argparse
import bisect
import csv
import datetime as dt
import re
import statistics
import sys
from pathlib import Path

CMD_EVENTS = {"pause", "play", "seek", "load", "auto_next", "volume"}


# --------------------------------------------------------------------------
# Okuma
# --------------------------------------------------------------------------

def load_trace(path):
    rows = []
    with open(path, newline="", encoding="utf-8-sig") as f:
        for idx, d in enumerate(csv.DictReader(f)):
            try:
                arg = (d.get("arg_ms") or "").strip()  # 8. sütun (Kasım 7 sonrası izlerde)
                rows.append({
                    "t": float(d["t_unix_ms"]),
                    "ev": d["event"].strip(),
                    "q": float(d["queued_ms"]),
                    "xr": int(float(d["xruns"])),
                    "gain": float(d["gain"]),
                    "target": float(d["target_ms"]),
                    "under": float(d["underrun_ms"]),
                    "arg": float(arg) if arg else None,
                    "idx": idx,
                })
            except (KeyError, ValueError, TypeError, AttributeError):
                continue  # yarım yazılmış son satır vb.
    rows.sort(key=lambda r: (r["t"], r["idx"]))
    return rows


# Windows Sistem günlüğünde bir kesintiyi açıklayabilecek olaylar: NTFS onarımı
# (130 / 132 / 133), chkdsk, gölge kopya (volsnap), güç (Kernel-Power), disk ve
# depolama sürücüsü uyarıları. 7 Kasım: 2,65 sn'lik takılma, temiz olmayan
# kapanıştan sonraki NTFS kendi kendini onarma + VSS + chkdsk spot-verify'dı.
NTFS_IDS = {130, 132, 133}
EVENT_PROVIDERS = r"(?i)ntfs|chkdsk|volsnap|kernel-power|^disk$|storport|stornvme|storahci|iastor|partmgr|volmgr"
DISK_RE = re.compile(r"(?i)ntfs|chkdsk|volsnap|disk|stor|iastor|partmgr|volmgr")


def windows_events(lo_ms, hi_ms):
    """[lo, hi] aralığındaki ilgili Sistem günlüğü olayları (tek PowerShell
    çağrısı). Windows değilse / okunamazsa (None, neden)."""
    import json
    import shutil
    import subprocess
    ps = shutil.which("powershell") or shutil.which("pwsh")
    if not ps:
        return None, "PowerShell yok"
    script = (
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8;"
        f"$s=[DateTimeOffset]::FromUnixTimeMilliseconds({int(lo_ms)}).LocalDateTime;"
        f"$e=[DateTimeOffset]::FromUnixTimeMilliseconds({int(hi_ms)}).LocalDateTime;"
        "$ev=Get-WinEvent -FilterHashtable @{LogName='System';StartTime=$s;EndTime=$e} -ErrorAction SilentlyContinue |"
        f" Where-Object {{ $_.ProviderName -match '{EVENT_PROVIDERS}' }} |"
        " ForEach-Object { [pscustomobject]@{ t=[DateTimeOffset]::new($_.TimeCreated).ToUnixTimeMilliseconds();"
        " p=$_.ProviderName; id=$_.Id; lvl=[int]$_.Level;"
        " msg=if ($_.Message) { ($_.Message -split \"`r?`n\")[0] } else { '' } } };"
        "@($ev) | ConvertTo-Json -Compress -Depth 2"
    )
    try:
        out = subprocess.run([ps, "-NoProfile", "-NonInteractive", "-Command", script],
                             capture_output=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired) as e:
        return None, f"PowerShell çalışmadı: {e}"
    text = out.stdout.decode("utf-8", errors="replace").strip()
    if not text:
        return [], None
    try:
        data = json.loads(text)
    except ValueError:
        return None, f"PowerShell çıktısı okunamadı: {text[:120]}"
    if isinstance(data, dict):
        data = [data]
    events = []
    for d in data or []:
        try:
            ev = {"t": float(d["t"]), "p": str(d["p"]), "id": int(d["id"]), "lvl": int(d.get("lvl") or 4),
                  "msg": str(d.get("msg") or "")}
        except (KeyError, TypeError, ValueError):
            continue
        # NTFS: yalnız onarım olayları ve uyarı / hatalar (bilgi gürültüsü atılır).
        if "ntfs" in ev["p"].lower() and ev["id"] not in NTFS_IDS and ev["lvl"] > 3:
            continue
        events.append(ev)
    events.sort(key=lambda e: e["t"])
    return events, None


def local_hms(t_ms):
    return dt.datetime.fromtimestamp(t_ms / 1000.0).strftime("%H:%M:%S.%f")[:-3]


def load_marks(path):
    marks = {}
    try:
        with open(path, newline="", encoding="utf-8-sig") as f:
            for d in csv.DictReader(f):
                try:
                    marks[d["phase"].strip()] = (float(d["start_unix_ms"]), float(d["end_unix_ms"]))
                except (KeyError, ValueError, TypeError):
                    continue
    except OSError:
        pass
    return marks


LOG_TS = re.compile(r"^\[?(\d{4}-\d\d-\d\d)T(\d\d):(\d\d):(\d\d)(?:\.(\d+))?Z")
PAUSE_RE = re.compile(r"Duraklat\S*:\s*\S*alar\s+(\d+)\s*ms,\s*duyulan\s+(\d+)\s*ms")
RESUME_RE = re.compile(r"\bDevam:\s*\S*alar\s+(\d+)\s*ms,\s*duyulan\s+(\d+)\s*ms")
XRUN_RE = re.compile(r"Ses \S+ bo\S+ \(xrun #(\d+)")
DEVDELAY_RE = re.compile(r"gecikmesi \(tahmin\) (\d+) ms")
TARGET_RE = re.compile(r"kuyruk hedefi (\d+) ms")


def log_ts_ms(line):
    m = LOG_TS.match(line)
    if not m:
        return None
    day, hh, mm, ss, frac = m.groups()
    y, mo, d = (int(x) for x in day.split("-"))
    base = dt.datetime(y, mo, d, int(hh), int(mm), int(ss), tzinfo=dt.timezone.utc)
    ms = int((frac or "0").ljust(3, "0")[:3])
    return base.timestamp() * 1000.0 + ms


def load_log(path, lo, hi):
    """Zaman damgası [lo, hi] dışında kalan satırlar atılır (başka oturum / örnek)."""
    lines = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            ts = log_ts_ms(line)
            if ts is not None and lo is not None and not (lo <= ts <= hi):
                continue
            lines.append(line.rstrip("\n"))
    return lines


# --------------------------------------------------------------------------
# Sonuç biçimi
# --------------------------------------------------------------------------

class Report:
    def __init__(self):
        self.items = []  # (durum, ad, ayrıntı)

    def add(self, ok, name, detail):
        self.items.append(("PASS" if ok else "FAIL", name, detail))

    def info(self, name, detail):
        self.items.append(("INFO", name, detail))

    def note(self, text):
        self.items.append(("NOT", text, ""))

    def print(self):
        for status, name, detail in self.items:
            if status == "NOT":
                print(f"[NOT ] {name}")
            else:
                print(f"[{status}] {name}" + (f": {detail}" if detail else ""))
        fails = [n for s, n, _ in self.items if s == "FAIL"]
        passes = sum(1 for s, _, _ in self.items if s == "PASS")
        print()
        print(f"SONUÇ: {passes} PASS, {len(fails)} FAIL" + (f" ({'; '.join(fails)})" if fails else ""))
        return 1 if fails else 0


def fmt_list(vals, limit=12):
    shown = " ".join(f"{v:.1f}" for v in vals[:limit])
    return f"[{shown}{' ...' if len(vals) > limit else ''}]"


def stats_text(vals, missing):
    if not vals:
        return f"örnek yok (eşleşmeyen {missing})"
    return (f"n={len(vals)}, eşleşmeyen {missing}, maks {max(vals):.1f}, "
            f"medyan {statistics.median(vals):.1f}, ort {statistics.fmean(vals):.1f} ms {fmt_list(vals)}")


# --------------------------------------------------------------------------
# Eşleştirme
# --------------------------------------------------------------------------

class Index:
    def __init__(self, rows):
        self.rows = rows
        self.by_ev = {}
        for r in rows:
            self.by_ev.setdefault(r["ev"], []).append(r["t"])
        self.ticks = [r for r in rows if r["ev"] == "tick"]
        self.tick_t = [r["t"] for r in self.ticks]

    def times(self, ev):
        return self.by_ev.get(ev, [])

    def first_after(self, ev, t, tol, timeout):
        """t'den (tol kadar önce de olabilir) sonraki ilk `ev` anı; timeout içinde."""
        ts = self.times(ev)
        i = bisect.bisect_left(ts, t - tol)
        if i < len(ts) and ts[i] - t <= timeout:
            return ts[i]
        return None

    def ticks_between(self, lo, hi):
        a = bisect.bisect_left(self.tick_t, lo)
        b = bisect.bisect_right(self.tick_t, hi)
        return self.ticks[a:b]

    def tick_at_or_before(self, t):
        i = bisect.bisect_right(self.tick_t, t) - 1
        return self.ticks[i] if i >= 0 else None


def pair_latency(ix, cmd, target, args):
    vals, missing = [], 0
    for t in ix.times(cmd):
        got = ix.first_after(target, t, args.clock_tol_ms, args.pair_timeout_ms)
        if got is None:
            missing += 1
        else:
            vals.append(max(0.0, got - t))
    return vals, missing


def volume_pairs(ix, args):
    """Her ses düzeyi komutu için, ses iş parçacığının AYNI değeri uyguladığı
    ilk `gain` anına kadar geçen süre. Sürüklemede komutlar birkaç ms arayla
    gelir; ses iş parçacığı geri çağrı başına yalnız son değeri uygular, aradaki
    değerler hiç uygulanmaz. Bir sonraki komut, bu değer uygulanmadan geldiyse
    komut "birleştirildi" sayılır (gecikme değil). 7 Kasım oturumunda kullanıcı
    çubuğu sürüklerken tek tek eşleştirme 103 ms "gecikme" göstermişti."""
    vols = [r for r in ix.rows if r["ev"] == "volume"]
    gains = [r for r in ix.rows if r["ev"] == "gain"]
    gain_t = [r["t"] for r in gains]
    vals, missing, merged = [], 0, 0
    for k, r in enumerate(vols):
        t, g = r["t"], r["gain"]
        nxt = vols[k + 1]["t"] if k + 1 < len(vols) else None
        i = bisect.bisect_left(gain_t, t - args.clock_tol_ms)
        got = None
        while i < len(gains) and gains[i]["t"] - t <= args.pair_timeout_ms:
            if nxt is not None and gains[i]["t"] > nxt + args.clock_tol_ms:
                break
            if abs(gains[i]["gain"] - g) < 1e-4:
                got = gains[i]["t"]
                break
            i += 1
        if got is not None:
            vals.append(max(0.0, got - t))
        elif nxt is not None and nxt - t <= args.pair_timeout_ms:
            merged += 1
        else:
            missing += 1
    return vals, missing, merged


def seek_pairs(ix, args):
    """Her sarma için (sessiz, duyulur) gecikmeleri; duyulur, sessizden sonra aranır."""
    silent, audible, miss_s, miss_a = [], [], 0, 0
    for t in ix.times("seek"):
        s = ix.first_after("silent", t, args.clock_tol_ms, args.pair_timeout_ms)
        if s is None:
            miss_s += 1
            miss_a += 1
            continue
        silent.append(max(0.0, s - t))
        a = ix.first_after("audible", s, 0.0, args.pair_timeout_ms * 3)
        if a is None:
            miss_a += 1
        else:
            audible.append(max(0.0, a - t))
    return silent, audible, miss_s, miss_a


def limit_check(rep, name, vals, missing, limit, need, args, extra=0.0, strict=False, unit_note=""):
    ok_n = len(vals) >= need and missing == 0
    if strict:
        ok_v = all(v + extra < limit for v in vals)
        cond = f"iç + D({extra:.0f}) < {limit:g} ms"
    else:
        ok_v = all(v + extra <= limit for v in vals)
        cond = f"<= {limit:g} ms" if extra == 0 else f"iç + D({extra:.0f}) <= {limit:g} ms"
    worst = f"; en kötü iç+D {max(vals) + extra:.1f}" if vals and extra else ""
    why = ""
    if not ok_n:
        why = f"; YETERSİZ/EKSİK örnek (gereken >= {need}, eşleşmeyen {missing})"
    rep.add(ok_n and ok_v, f"{name} {cond}", stats_text(vals, missing) + worst + why + unit_note)


# --------------------------------------------------------------------------
# Ana akış
# --------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("csv")
    ap.add_argument("log")
    ap.add_argument("--marks", default=None)
    ap.add_argument("--device-ms", type=float, default=30.0, help="varsayılan ses kartı gecikmesi D (30)")
    ap.add_argument("--pause-silent-ms", type=float, default=20.0)
    ap.add_argument("--pause-total-ms", type=float, default=60.0, help="iç + D bu değerin altında olmalı")
    ap.add_argument("--volume-gain-ms", type=float, default=11.0)
    ap.add_argument("--volume-total-ms", type=float, default=40.0)
    ap.add_argument("--seek-silent-ms", type=float, default=20.0)
    ap.add_argument("--seek-audible-total-ms", type=float, default=150.0)
    ap.add_argument("--target-ms", type=float, default=120.0)
    ap.add_argument("--target-tol-ms", type=float, default=0.5)
    ap.add_argument("--xrun-max", type=int, default=0)
    ap.add_argument("--underrun-max-ms", type=float, default=0.0)
    ap.add_argument("--paused-pos-tol-ms", type=float, default=2.0)
    ap.add_argument("--gapless-before-ms", type=float, default=500.0)
    # auto_next, şarkı sonu payıyla duyulan sınırdan ~4 sn önce gelir; takılma
    # bu payı aşarsa kesinti sınırda, yani auto_next'ten ~4+ sn sonra olur.
    ap.add_argument("--gapless-after-ms", type=float, default=6000.0)
    ap.add_argument("--min-queued-ms", type=float, default=0.0, help="geçiş penceresinde queued_ms bunun ÜSTÜNDE olmalı")
    ap.add_argument("--min-auto-next", type=int, default=1)
    ap.add_argument("--min-samples", type=int, default=5, help="duraklat / sarma / konum için en az örnek")
    ap.add_argument("--min-volume-samples", type=int, default=5)
    ap.add_argument("--min-soak-minutes", type=float, default=0.0)
    ap.add_argument("--pair-timeout-ms", type=float, default=1000.0)
    ap.add_argument("--clock-tol-ms", type=float, default=1.0, help="sessiz/duyulur/gain anı komuttan bu kadar önce görünürse de kabul")
    ap.add_argument("--no-log-time-filter", action="store_true")
    ap.add_argument("--skip-volume", action="store_true", help="sessiz koşu: ses düzeyi ölçütü ölçülmedi sayılır")
    ap.add_argument("--no-events", action="store_true", help="xrun'lar için Windows Sistem günlüğüne bakma")
    ap.add_argument("--event-window-ms", type=float, default=10_000.0, help="xrun çevresinde aranan olay penceresi (+-)")
    args = ap.parse_args()

    try:
        rows = load_trace(args.csv)
    except OSError as e:
        print(f"iz okunamadı: {e}", file=sys.stderr)
        return 2
    if not rows:
        print("iz boş (başlık dışında satır yok)", file=sys.stderr)
        return 2
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")

    ix = Index(rows)
    t_first, t_last = rows[0]["t"], rows[-1]["t"]
    marks_path = args.marks
    if marks_path is None:
        cand = Path(args.csv).with_suffix(".marks.csv")
        marks_path = str(cand) if cand.exists() else None
    marks = load_marks(marks_path) if marks_path else {}

    try:
        lo, hi = (None, None) if args.no_log_time_filter else (t_first - 5000, t_last + 5000)
        log = load_log(args.log, lo, hi)
    except OSError as e:
        print(f"log okunamadı: {e}", file=sys.stderr)
        return 2

    rep = Report()
    D = args.device_ms
    counts = {ev: len(ix.times(ev)) for ev in
              ("pause", "play", "seek", "volume", "auto_next", "load", "silent", "audible", "gain", "xrun")}
    rep.info("iz", f"{len(rows)} satır, {(t_last - t_first) / 60000:.1f} dk; " +
             ", ".join(f"{k}={v}" for k, v in counts.items()))

    # ---- log: aygıt bilgisi
    opened = [l for l in log if "gecikmesi (tahmin)" in l]
    if opened:
        dm = DEVDELAY_RE.search(opened[-1])
        tm = TARGET_RE.search(opened[-1])
        rep.info("ses çıkışı (log)", f"aygıt gecikmesi tahmini {dm.group(1) if dm else '?'} ms (varsayılan D={D:g}), "
                 f"kuyruk hedefi {tm.group(1) if tm else '?'} ms")
    else:
        rep.note("logda 'Ses çıkışı açıldı' satırı yok: RUST_LOG openify=debug (en az info) mü, dilim doğru mu?")

    # ---- duraklat -> sessiz
    v, m = pair_latency(ix, "pause", "silent", args)
    limit_check(rep, "duraklat->sessiz (iç)", v, m, args.pause_silent_ms, args.min_samples, args)
    limit_check(rep, "duraklat->sessiz", v, m, args.pause_total_ms, args.min_samples, args, extra=D, strict=True)

    # ---- ses düzeyi -> gain
    v, m, merged = volume_pairs(ix, args)
    mnote = f"; birleştirilen (sürükleme) {merged}" if merged else ""
    if args.skip_volume:
        rep.note("ses düzeyi->gain: ölçülmedi (sessiz koşu)")
    else:
        limit_check(rep, "ses düzeyi->gain (iç)", v, m, args.volume_gain_ms, args.min_volume_samples, args, unit_note=mnote)
        limit_check(rep, "ses düzeyi->gain", v, m, args.volume_total_ms, args.min_volume_samples, args, extra=D,
                    strict=True, unit_note=mnote)

    # ---- sarma
    s_silent, s_audible, ms_, ma_ = seek_pairs(ix, args)
    limit_check(rep, "sarma->sessiz", s_silent, ms_, args.seek_silent_ms, args.min_samples, args)
    limit_check(rep, "sarma->duyulur", s_audible, ma_, args.seek_audible_total_ms, args.min_samples, args,
                extra=D, strict=True)

    # ---- bilgi: devam -> duyulur
    v, m = pair_latency(ix, "play", "audible", args)
    rep.info("devam->duyulur (iç; ölçüt değil)", stats_text(v, m))

    # ---- soak
    ticks_all = ix.ticks
    if not ticks_all:
        rep.add(False, "soak", "izde hiç tick satırı yok")
    else:
        if "soak" in marks:
            s0, s1 = marks["soak"]
            src = "faz dosyasından"
        else:
            s0, s1 = ticks_all[0]["t"], ticks_all[-1]["t"]
            src = "faz dosyası YOK: bütün iz"
            rep.note("faz dosyası bulunamadı; soak penceresi = bütün iz (pause/volume/seek dahil)")
        win = ix.ticks_between(s0, s1)
        base = ix.tick_at_or_before(s0) or ticks_all[0]
        if len(win) < 2:
            rep.add(False, "soak pencere", f"pencerede {len(win)} tick ({src})")
        else:
            minutes = (win[-1]["t"] - win[0]["t"]) / 60000.0
            dx = win[-1]["xr"] - base["xr"]
            du = win[-1]["under"] - base["under"]
            tmin = min(r["target"] for r in win)
            tmax = max(r["target"] for r in win)
            qmin = min(r["q"] for r in win)
            qavg = statistics.fmean(r["q"] for r in win)
            xr_rows = [t for t in ix.times("xrun") if s0 <= t <= s1]
            rep.add(minutes >= args.min_soak_minutes, f"soak süresi >= {args.min_soak_minutes:g} dk",
                    f"{minutes:.1f} dk ({src}), {len(win)} tick")
            rep.add(dx <= args.xrun_max and not xr_rows, f"soak xrun <= {args.xrun_max}",
                    f"{dx} (xrun olay satırı {len(xr_rows)})")
            rep.add(du <= args.underrun_max_ms + 1e-6, f"soak underrun_ms <= {args.underrun_max_ms:g}", f"{du:.1f} ms")
            rep.add(abs(tmin - args.target_ms) <= args.target_tol_ms and abs(tmax - args.target_ms) <= args.target_tol_ms,
                    f"soak target_ms sabit {args.target_ms:g}", f"min {tmin:.0f}, maks {tmax:.0f} ms")
            rep.info("soak kuyruk (queued_ms)", f"min {qmin:.1f}, ort {qavg:.1f} ms")

    # ---- boşluksuz geçiş
    autos = ix.times("auto_next")
    gap_ok_all = len(autos) >= args.min_auto_next
    details = []
    for t in autos:
        a, b = t - args.gapless_before_ms, t + args.gapless_after_ms
        w = ix.ticks_between(a, b)
        if len(w) < 3:
            gap_ok_all = False
            details.append(f"t={t:.0f}: pencerede yalnız {len(w)} tick (yetersiz veri)")
            continue
        dx = w[-1]["xr"] - w[0]["xr"]
        du = w[-1]["under"] - w[0]["under"]
        xr_rows = [x for x in ix.times("xrun") if a <= x <= b]
        qmin = min(r["q"] for r in w)
        ok = dx == 0 and du <= 1e-6 and not xr_rows and qmin > args.min_queued_ms
        gap_ok_all = gap_ok_all and ok
        details.append(f"{'ok' if ok else 'HATA'}(xrun+{dx}, underrun+{du:.1f}, q_min {qmin:.1f})")
    rep.add(gap_ok_all, f"boşluksuz geçiş: {len(autos)} auto_next, pencere -{args.gapless_before_ms:g}/+{args.gapless_after_ms:g} ms, "
            f"xrun yok, queued_ms > {args.min_queued_ms:g}",
            "; ".join(details) if details else f"auto_next yok (gereken >= {args.min_auto_next})")

    # ---- duraklatılmış konum (log)
    pairs, last_pause = [], None
    for line in log:
        mp = PAUSE_RE.search(line)
        if mp:
            last_pause = int(mp.group(2))
            continue
        mr = RESUME_RE.search(line)
        if mr and last_pause is not None:
            pairs.append((last_pause, int(mr.group(2))))
            last_pause = None
    devs = [abs(b - a) for a, b in pairs]
    ok = len(pairs) >= args.min_samples and all(d <= args.paused_pos_tol_ms for d in devs)
    detail = (f"n={len(pairs)}, maks sapma {max(devs) if devs else 0} ms "
              f"[{' '.join(f'{a}->{b}' for a, b in pairs[:8])}{' ...' if len(pairs) > 8 else ''}]")
    if len(pairs) < args.min_samples:
        detail += f"; YETERSİZ çift (gereken >= {args.min_samples})"
    rep.add(ok, f"duraklatılmış konum: devam duyulan == duraklat duyulan (+-{args.paused_pos_tol_ms:g} ms)", detail)

    # ---- geçiş: yeni şarkının ilk paketi (first_write) ne kadar sonra geldi
    fws = ix.times("first_write")
    if not fws:
        rep.note("izde first_write satırı yok (eski iz ya da geçiş yok): ilk paket gecikmesi ölçülmedi")
    else:
        lat, missing, parts = [], 0, []
        for t in autos:
            got = ix.first_after("first_write", t, 0.0, 10_000.0)
            if got is None:
                missing += 1
                parts.append(f"{local_hms(t)}: first_write YOK")
                continue
            row = next(r for r in rows if r["t"] == got and r["ev"] == "first_write")
            lat.append(got - t)
            parts.append(f"{local_hms(t)}: {got - t:.0f} ms (kuyrukta kalan {row['q']:.0f} ms)")
        rep.info("auto_next -> first_write (ilk paket gecikmesi; pay ~4000 ms)",
                 stats_text(lat, missing) + ("; " + "; ".join(parts) if parts else ""))

    # ---- üretici boşlukları (çalarken çözücü iki paket arası > 200 ms)
    gaps = [r for r in rows if r["ev"] == "producer_gap"]
    if not any(r["ev"] in ("first_write", "producer_gap", "tail_on") for r in rows):
        rep.note("eski iz (first_write / producer_gap satırları yok): üretici boşlukları ölçülmedi")
    elif not gaps:
        rep.info("producer_gap (> 200 ms)", "yok")
    else:
        vals = [r["arg"] for r in gaps if r["arg"] is not None]
        listed = "; ".join(f"{local_hms(r['t'])}: {r['arg'] or 0:.0f} ms (kuyruk {r['q']:.0f} ms)" for r in gaps[:12])
        rep.info("producer_gap (> 200 ms)", f"n={len(gaps)}, maks {max(vals) if vals else 0:.0f} ms; {listed}"
                 + (" ..." if len(gaps) > 12 else ""))

    # ---- her xrun (ve 1 sn'yi aşan üretici boşluğu) için Windows Sistem günlüğü
    xr_rows = [r for r in rows if r["ev"] == "xrun"]
    big_gaps = [r for r in gaps if (r["arg"] or 0) >= 1000.0]
    incidents = [("xrun", r) for r in xr_rows] + [("producer_gap", r) for r in big_gaps]
    if incidents and not args.no_events:
        win = args.event_window_ms
        lo = min(r["t"] for _, r in incidents) - win - 5000
        hi = max(r["t"] for _, r in incidents) + win
        events, why = windows_events(lo, hi)
        prev_under = 0.0
        for kind, r in sorted(incidents, key=lambda x: x[1]["t"]):
            if kind == "xrun":
                silence = r["under"] - prev_under
                prev_under = r["under"]
                what = f"xrun @ {local_hms(r['t'])} (sessizlik ~{silence:.0f} ms)"
            else:
                what = f"producer_gap @ {local_hms(r['t'])} ({r['arg']:.0f} ms)"
            if events is None:
                rep.info(what, f"Sistem günlüğü okunamadı ({why})")
                continue
            near = [e for e in events if abs(e["t"] - r["t"]) <= win]
            disk = [e for e in near if DISK_RE.search(e["p"])]
            label = "DİSK OLAYI" if disk else ("güç olayı" if near else f"±{win / 1000:g} sn içinde disk / güç olayı yok")
            detail = "; ".join(f"{local_hms(e['t'])} {e['p']} {e['id']}: {e['msg'][:90]}" for e in near[:8])
            rep.info(what, label + (f": {detail}" if detail else ""))
    elif incidents:
        rep.note("Sistem günlüğü sorgusu kapalı (--no-events)")

    # ---- ek: oturum boyunca xrun; logda xrun uyarısı
    last_tick = ticks_all[-1] if ticks_all else None
    if last_tick:
        rep.add(last_tick["xr"] <= args.xrun_max and last_tick["under"] <= args.underrun_max_ms + 1e-6,
                "(ek) oturum boyunca xrun / underrun toplamı",
                f"xrun {last_tick['xr']}, underrun {last_tick['under']:.1f} ms")
    log_xr = [l for l in log if XRUN_RE.search(l)]
    rep.add(not log_xr, "(ek) logda 'Ses çıkışı boşaldı' uyarısı yok",
            f"{len(log_xr)} satır" + (f", ilki: {log_xr[0][:160]}" if log_xr else ""))

    return rep.print()


if __name__ == "__main__":
    sys.exit(main())
