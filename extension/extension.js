import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const IFACE_XML = `
<node>
  <interface name="org.gnome.Shell.Extensions.FocusByPid">
    <method name="ActivateByPid">
      <arg type="u" direction="in" name="pid"/>
      <arg type="b" direction="out" name="success"/>
    </method>
    <method name="HighlightByPid">
      <arg type="u" direction="in" name="pid"/>
      <arg type="u" direction="in" name="duration_ms"/>
      <arg type="b" direction="out" name="success"/>
    </method>
    <method name="HighlightBySession">
      <arg type="s" direction="in" name="marker"/>
      <arg type="u" direction="in" name="pid"/>
      <arg type="u" direction="in" name="duration_ms"/>
      <arg type="b" direction="out" name="found"/>
      <arg type="b" direction="out" name="already_focused"/>
    </method>
  </interface>
</node>`;

const BORDER_WIDTH = 3;
const BORDER_COLOR = '#00ff41';
const FADE_DURATION = 300;

export default class FocusByPidExtension {
    _dbusId = null;
    _highlights = [];
    _timeouts = [];

    enable() {
        this._impl = Gio.DBusExportedObject.wrapJSObject(IFACE_XML, this);
        this._impl.export(
            Gio.DBus.session,
            '/org/gnome/Shell/Extensions/FocusByPid'
        );

        this._dbusId = Gio.DBus.session.own_name(
            'org.gnome.Shell.Extensions.FocusByPid',
            Gio.BusNameOwnerFlags.NONE,
            null,
            null
        );
    }

    disable() {
        this._clearHighlights();
        if (this._impl) {
            this._impl.unexport();
            this._impl = null;
        }
        if (this._dbusId) {
            Gio.DBus.session.unown_name(this._dbusId);
            this._dbusId = null;
        }
    }

    _clearHighlights() {
        for (const id of this._timeouts)
            GLib.source_remove(id);
        this._timeouts = [];

        for (const borders of this._highlights) {
            for (const b of borders)
                b.destroy();
        }
        this._highlights = [];
    }

    ActivateByPid(pid) {
        const win = this._findBestWindowByPid(pid);
        if (!win) return false;

        const workspace = win.get_workspace();
        const activeWorkspace = global.workspace_manager.get_active_workspace();

        if (workspace && workspace !== activeWorkspace) {
            workspace.activate(global.get_current_time());
        }

        Main.activateWindow(win);
        return true;
    }

    HighlightByPid(pid, duration_ms) {
        const win = this._findBestWindowByPid(pid);
        if (!win) return false;

        const workspace = win.get_workspace();
        const activeWorkspace = global.workspace_manager.get_active_workspace();

        // Switch workspace first only if the window lives on another one...
        if (workspace && workspace !== activeWorkspace) {
            workspace.activate(global.get_current_time());
        }
        // ...then ALWAYS raise/activate it. Previously this was inside the
        // cross-workspace branch, so a same-workspace window got a border but
        // was never raised (contradicting the README). ActivateByPid already
        // calls activateWindow unconditionally — this mirrors it.
        Main.activateWindow(win);

        this._highlightWindow(win, duration_ms || 3000);
        return true;
    }

    HighlightBySession(marker, pid, duration_ms) {
        const win = this._resolveWindow(marker, pid);
        if (!win) return [false, false];

        // 4.3a: if it's already the focused window, do nothing (no flash).
        if (global.display.get_focus_window() === win) {
            return [true, true];
        }

        const workspace = win.get_workspace();
        const activeWorkspace = global.workspace_manager.get_active_workspace();
        if (workspace && workspace !== activeWorkspace) {
            workspace.activate(global.get_current_time());
        }
        Main.activateWindow(win);
        this._highlightWindow(win, duration_ms || 3000);
        return [true, false];
    }

    _resolveWindow(marker, pid) {
        // 1. Precise: a window whose title carries this session's marker.
        if (marker) {
            const tagged = global.get_window_actors()
                .map(actor => actor.get_meta_window())
                .filter(win => {
                    if (!win) return false;
                    const title = win.get_title();
                    return title !== null && title.includes(marker);
                });
            if (tagged.length > 0) {
                tagged.sort((a, b) => b.get_user_time() - a.get_user_time());
                return tagged[0];
            }
        }
        // 2. Fallback: today's PID + most-recently-focused selection.
        return this._findBestWindowByPid(pid);
    }

    _findBestWindowByPid(pid) {
        const actors = global.get_window_actors();
        const matches = [];
        for (const actor of actors) {
            const win = actor.get_meta_window();
            if (win && win.get_pid() === pid) {
                matches.push(win);
            }
        }
        if (matches.length === 0) return null;
        // Sort by most recently focused first
        matches.sort((a, b) => b.get_user_time() - a.get_user_time());
        return matches[0];
    }

    _highlightWindow(win, duration_ms) {
        const rect = win.get_frame_rect();
        const bw = BORDER_WIDTH;

        const borders = [
            // Top
            this._createBorder(rect.x - bw, rect.y - bw, rect.width + 2 * bw, bw),
            // Bottom
            this._createBorder(rect.x - bw, rect.y + rect.height, rect.width + 2 * bw, bw),
            // Left
            this._createBorder(rect.x - bw, rect.y, bw, rect.height),
            // Right
            this._createBorder(rect.x + rect.width, rect.y, bw, rect.height),
        ];

        this._highlights.push(borders);

        const timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, duration_ms, () => {
            for (const border of borders) {
                border.ease({
                    opacity: 0,
                    duration: FADE_DURATION,
                    onComplete: () => border.destroy(),
                });
            }
            const idx = this._highlights.indexOf(borders);
            if (idx >= 0) this._highlights.splice(idx, 1);
            const tidx = this._timeouts.indexOf(timeoutId);
            if (tidx >= 0) this._timeouts.splice(tidx, 1);
            return GLib.SOURCE_REMOVE;
        });
        this._timeouts.push(timeoutId);
    }

    _createBorder(x, y, width, height) {
        const border = new St.Bin({
            style: `background-color: ${BORDER_COLOR};`,
            x, y, width, height,
            reactive: false,
        });
        Main.layoutManager.uiGroup.add_child(border);
        return border;
    }
}
