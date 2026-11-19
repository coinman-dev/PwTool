#!/usr/bin/env python3
"""Minimal org.kde.StatusNotifierWatcher for headless tests of the tray icon
(the virtual KWin session has no Plasma panel). Registers items and prints
them; run inside tools/headless.sh."""
import sys
from gi.repository import Gio, GLib

XML = """
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
    <method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
  </interface>
</node>"""
items = []

def on_call(conn, sender, path, iface, method, params, inv):
    if method == "RegisterStatusNotifierItem":
        svc = params.unpack()[0]
        if svc.startswith("/"):
            svc = sender + svc
        items.append(svc)
        print("registered", svc, flush=True)
    inv.return_value(None)

def on_get(conn, sender, path, iface, prop):
    if prop == "RegisteredStatusNotifierItems":
        return GLib.Variant("as", items)
    if prop == "IsStatusNotifierHostRegistered":
        return GLib.Variant("b", True)
    return GLib.Variant("i", 0)

def on_bus(conn, name):
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object(  # noqa: deprecated but available
        "/StatusNotifierWatcher", node.interfaces[0], on_call, on_get, None)

Gio.bus_own_name(Gio.BusType.SESSION, "org.kde.StatusNotifierWatcher", 0, on_bus, None, None)
GLib.timeout_add_seconds(int(sys.argv[1]) if len(sys.argv) > 1 else 30, lambda: sys.exit(0))
GLib.MainLoop().run()
