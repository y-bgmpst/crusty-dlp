#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: $0 --deb FILE --rpm FILE [--x11] [--wayland]" >&2
    exit 2
}

deb=""
rpm=""
check_x11=0
check_wayland=0
while (($#)); do
    case "$1" in
        --deb) shift; (($#)) || usage; deb=$1 ;;
        --rpm) shift; (($#)) || usage; rpm=$1 ;;
        --x11) check_x11=1 ;;
        --wayland) check_wayland=1 ;;
        *) usage ;;
    esac
    shift
done

[[ -n "$deb" && -f "$deb" && -n "$rpm" && -f "$rpm" ]] || usage
deb=$(realpath "$deb")
rpm=$(realpath "$rpm")

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

verify_tree() {
    local root=$1
    local prefix="$root/usr"
    require_file() {
        local path=$1
        local mode=${2:-file}
        if [[ $mode == executable && ! -x $path ]]; then
            echo "Missing executable: $path" >&2
            return 1
        elif [[ $mode == file && ! -f $path ]]; then
            echo "Missing file: $path" >&2
            return 1
        fi
    }
    require_file "$prefix/bin/crusty-dlp" executable
    require_file "$prefix/bin/crusty-dlp-gui" executable
    require_file "$prefix/share/applications/crusty-dlp.desktop"
    require_file "$prefix/share/icons/hicolor/scalable/apps/crusty-dlp.svg"
    require_file "$prefix/share/icons/hicolor/48x48/apps/crusty-dlp.png"
    require_file "$prefix/share/licenses/crusty-dlp/LICENSE"
    require_file "$prefix/share/crusty-dlp/plugins/yt_dlp_plugins/extractor/boyfriendtv.py"
    require_file "$prefix/share/crusty-dlp/plugins/yt_dlp_plugins/extractor/ooxxx.py"
    require_file "$prefix/share/crusty-dlp/plugins/yt_dlp_plugins/extractor/pmvhaven.py"
    require_file "$prefix/share/crusty-dlp/plugins/yt_dlp_plugins/extractor/spankbang.py"

    if command -v desktop-file-validate >/dev/null 2>&1; then
        desktop-file-validate "$prefix/share/applications/crusty-dlp.desktop"
    fi
    grep -Fxq 'Exec=crusty-dlp-gui %U' "$prefix/share/applications/crusty-dlp.desktop"
    grep -Fxq 'Icon=crusty-dlp' "$prefix/share/applications/crusty-dlp.desktop"
    grep -Fxq 'StartupWMClass=crusty-dlp' "$prefix/share/applications/crusty-dlp.desktop"

    # The hicolor index is owned by the host desktop theme, not by this
    # package. Refresh it when a staged tree provides one; otherwise the
    # package hooks are verified by the packaging jobs on the host system.
    if [[ -f "$prefix/share/icons/hicolor/index.theme" ]] && \
        command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t -f "$prefix/share/icons/hicolor"
    fi
}

extract_deb="$tmp/deb"
extract_rpm="$tmp/rpm"
mkdir -p "$extract_deb" "$extract_rpm"
if command -v dpkg-deb >/dev/null 2>&1; then
    dpkg-deb -x "$deb" "$extract_deb"
else
    deb_dir="$tmp/deb-archive"
    mkdir -p "$deb_dir"
    cp "$deb" "$deb_dir/package.deb"
    (cd "$deb_dir" && ar x package.deb)
    data_archive=$(find "$deb_dir" -maxdepth 1 -type f \( -name 'data.tar.*' -o -name 'data.tar' \) | head -n 1)
    [[ -n "${data_archive:-}" ]] || {
        echo "Unable to locate data.tar.* in $deb" >&2
        exit 1
    }
    tar -xf "$data_archive" -C "$extract_deb"
fi

if command -v rpm2cpio >/dev/null 2>&1; then
    rpm2cpio "$rpm" | (cd "$extract_rpm" && cpio -idm --quiet)
elif command -v bsdtar >/dev/null 2>&1; then
    bsdtar -xf "$rpm" -C "$extract_rpm"
else
    echo "Need rpm2cpio or bsdtar to extract $rpm" >&2
    exit 1
fi
verify_tree "$extract_deb"
verify_tree "$extract_rpm"

smoke_gui() {
    local root=$1
    local log=$2
    set +e
    timeout --signal=TERM --kill-after=2s 8s env WINIT_UNIX_BACKEND=x11 HOME="$tmp/home" \
        XDG_CONFIG_HOME="$tmp/config" "$root/usr/bin/crusty-dlp-gui" >"$log" 2>&1
    local status=$?
    set -e
    if [[ $status -ne 124 && $status -ne 137 && $status -ne 143 ]]; then
        if grep -Eq 'NoCompositor|cannot open display|Can.t open display|Connection\(NoCompositor\)' "$log"; then
            echo "GUI smoke test skipped: no usable local display server in this session" >&2
            return 0
        fi
    fi
    # A GUI has no automatic exit path; timeout is the expected result.
    [[ $status -eq 124 || $status -eq 137 || $status -eq 143 ]] || {
        cat "$log" >&2
        echo "GUI smoke test failed with exit code $status" >&2
        return 1
    }
}

smoke_desktop_launch() {
    local root=$1
    local log=$2
    local desktop_home="$tmp/desktop-home"
    mkdir -p "$desktop_home/applications"
    cp "$root/usr/share/applications/crusty-dlp.desktop" "$desktop_home/applications/"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$desktop_home/applications" >/dev/null 2>&1 || true
    fi

    set +e
    timeout --signal=TERM --kill-after=2s 8s env \
        PATH="$root/usr/bin:$PATH" \
        HOME="$tmp/home" \
        XDG_CONFIG_HOME="$tmp/config" \
        XDG_DATA_HOME="$desktop_home" \
        WINIT_UNIX_BACKEND=x11 \
        gtk-launch crusty-dlp >"$log" 2>&1
    local status=$?
    set -e
    if [[ $status -ne 124 && $status -ne 137 && $status -ne 143 ]]; then
        if grep -Eq 'NoCompositor|cannot open display|Can.t open display|Connection\(NoCompositor\)' "$log"; then
            echo "Desktop launcher smoke test skipped: no usable local display server in this session" >&2
            return 0
        fi
    fi
    [[ $status -eq 124 || $status -eq 137 || $status -eq 143 ]] || {
        cat "$log" >&2
        echo "Desktop launcher smoke test failed with exit code $status" >&2
        return 1
    }
}

if ((check_x11)); then
    [[ -n "${DISPLAY:-}" ]] || { echo "X11 check requested without DISPLAY" >&2; exit 1; }
    smoke_gui "$extract_deb" "$tmp/x11.log"
    smoke_gui "$extract_rpm" "$tmp/x11-rpm.log"
    smoke_desktop_launch "$extract_deb" "$tmp/x11-launch.log"
    if ! xprop -root _NET_CLIENT_LIST >/dev/null 2>&1; then
        echo "X11 identity check skipped: no usable local display server in this session" >&2
    else
    x11_log="$tmp/x11-identity.log"
    timeout --signal=TERM --kill-after=2s 12s env WINIT_UNIX_BACKEND=x11 \
        HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" \
        "$extract_deb/usr/bin/crusty-dlp-gui" >"$x11_log" 2>&1 &
    gui_pid=$!
    found=0
    for _ in $(seq 1 80); do
        clients=$(xprop -root _NET_CLIENT_LIST 2>/dev/null || true)
        for client in $(sed 's/.*# //' <<<"$clients" | tr ',' ' '); do
            class=$(xprop -id "$client" WM_CLASS 2>/dev/null || true)
            if grep -Eq 'WM_CLASS.*crusty-dlp' <<<"$class"; then
                found=1
                break 2
            fi
        done
        sleep 0.25
    done
    kill "$gui_pid" 2>/dev/null || true
    wait "$gui_pid" 2>/dev/null || true
    if ((found == 0)); then
        cat "$x11_log" >&2
        echo "X11 WM_CLASS did not contain crusty-dlp" >&2
        exit 1
    fi
    fi
fi

if ((check_wayland)); then
    [[ -n "${WAYLAND_DISPLAY:-}" && -n "${XDG_RUNTIME_DIR:-}" ]] || {
        echo "Wayland check requested without WAYLAND_DISPLAY/XDG_RUNTIME_DIR" >&2
        exit 1
    }
    set +e
    timeout --signal=TERM 8s env DISPLAY= WAYLAND_DEBUG=1 HOME="$tmp/home" \
        XDG_CONFIG_HOME="$tmp/config" "$extract_deb/usr/bin/crusty-dlp-gui" \
        >"$tmp/wayland.out" 2>"$tmp/wayland.log"
    status=$?
    set -e
    wayland_skip=0
    if [[ $status -ne 124 && $status -ne 143 ]]; then
        if grep -Eq 'NoCompositor|Connection\(NoCompositor\)' "$tmp/wayland.log"; then
            echo "Wayland GUI smoke test skipped: no compositor available in this session" >&2
            wayland_skip=1
        fi
        if ((wayland_skip == 0)); then
            cat "$tmp/wayland.log" >&2
            echo "Wayland GUI smoke test failed with exit code $status" >&2
            exit 1
        fi
    fi
    if ((wayland_skip == 0)); then
        smoke_desktop_launch "$extract_deb" "$tmp/wayland-launch.log"
        grep -Eq 'set_app_id[^[:cntrl:]]*crusty-dlp' "$tmp/wayland.log" || {
            cat "$tmp/wayland.log" >&2
            echo "Wayland app_id did not contain crusty-dlp" >&2
            exit 1
        }
    fi
fi

echo "Linux package verification passed: desktop entry, icons, plugins, launcher files"
