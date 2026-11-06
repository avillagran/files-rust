// Typed, bounded libdbus bridge. The worker thread owns every opaque handle.
#include <dbus/dbus.h>
#include <cstddef>
#include <cstdio>
#include <cstring>
#include <new>

namespace {
constexpr const char *NAME = "org.freedesktop.FileManager1";
constexpr const char *PATH = "/org/freedesktop/FileManager1";
constexpr long MAX_WIRE = 64 * 1024 + 4096;
constexpr std::size_t MAX_URIS = 32;
constexpr std::size_t MAX_URI = 3 * 4096 + 16;
constexpr std::size_t MAX_TEXT = 64 * 1024;
constexpr const char *XML =
    "<node><interface name='org.freedesktop.FileManager1'>"
    "<method name='ShowFolders'><arg name='URIs' type='as' direction='in'/>"
    "<arg name='StartupId' type='s' direction='in'/></method>"
    "<method name='ShowItems'><arg name='URIs' type='as' direction='in'/>"
    "<arg name='StartupId' type='s' direction='in'/></method>"
    "<method name='ShowItemProperties'><arg name='URIs' type='as' direction='in'/>"
    "<arg name='StartupId' type='s' direction='in'/></method></interface>"
    "<interface name='org.freedesktop.DBus.Introspectable'><method name='Introspect'>"
    "<arg type='s' direction='out'/></method></interface>"
    "<interface name='org.freedesktop.DBus.Peer'><method name='Ping'/>"
    "<method name='GetMachineId'><arg type='s' direction='out'/></method>"
    "</interface></node>";

void diagnostic(char *out, std::size_t cap, const char *text) {
    if (out && cap) std::snprintf(out, cap, "%s", text ? text : "D-Bus operation failed");
}

bool send(DBusConnection *conn, DBusMessage *reply) {
    if (!reply) return false;
    const bool ok = dbus_connection_send(conn, reply, nullptr);
    dbus_message_unref(reply);
    return ok;
}

bool error(DBusConnection *conn, DBusMessage *message, const char *name, const char *text) {
    if (dbus_message_get_no_reply(message)) return true;
    return send(conn, dbus_message_new_error(message, name, text));
}

bool empty_reply(DBusConnection *conn, DBusMessage *message) {
    if (dbus_message_get_no_reply(message)) return true;
    return send(conn, dbus_message_new_method_return(message));
}

bool string_reply(DBusConnection *conn, DBusMessage *message, const char *text) {
    if (dbus_message_get_no_reply(message)) return true;
    DBusMessage *reply = dbus_message_new_method_return(message);
    if (!reply) return false;
    if (!dbus_message_append_args(reply, DBUS_TYPE_STRING, &text, DBUS_TYPE_INVALID)) {
        dbus_message_unref(reply);
        return false;
    }
    return send(conn, reply);
}

DBusMessage *bus_call(DBusConnection *conn, DBusMessage *message, DBusError *err) {
    if (!message) return nullptr;
    dbus_message_set_allow_interactive_authorization(message, FALSE);
    DBusMessage *reply = dbus_connection_send_with_reply_and_block(conn, message, 1000, err);
    dbus_message_unref(message);
    return reply;
}
} // namespace

struct FilesFMCall {
    DBusMessage *message;
    int method;
    std::size_t count;
    const char *uris[MAX_URIS];
    const char *startup;
};

extern "C" {
void *files_fm_open(const char *address, char *reason, std::size_t capacity) {
    if (!dbus_threads_init_default()) {
        diagnostic(reason, capacity, "Cannot initialize D-Bus threading");
        return nullptr;
    }
    DBusError err = DBUS_ERROR_INIT;
    DBusConnection *conn = dbus_connection_open_private(address, &err);
    if (!conn) {
        diagnostic(reason, capacity, err.message);
        dbus_error_free(&err);
        return nullptr;
    }
    dbus_connection_set_exit_on_disconnect(conn, FALSE);
    dbus_connection_set_max_message_size(conn, MAX_WIRE);
    dbus_connection_set_max_received_size(conn, MAX_WIRE * 10);
    dbus_connection_set_max_message_unix_fds(conn, 0);
    // The aggregate threshold must remain positive: libdbus pauses socket
    // reads when live_fds >= threshold, including 0 >= 0. The per-message
    // limit above still rejects every FD-bearing message.
    dbus_connection_set_max_received_unix_fds(conn, 1);

    // Explicit Hello and RequestName keep each startup call to one second;
    // the convenience bus helpers use the much longer default timeout.
    DBusMessage *reply = bus_call(conn, dbus_message_new_method_call(
        DBUS_SERVICE_DBUS, DBUS_PATH_DBUS, DBUS_INTERFACE_DBUS, "Hello"), &err);
    const char *unique = nullptr;
    if (!reply || !dbus_message_get_args(reply, &err, DBUS_TYPE_STRING, &unique,
                                        DBUS_TYPE_INVALID)
        || !dbus_bus_set_unique_name(conn, unique)) {
        diagnostic(reason, capacity, err.message ? err.message : "D-Bus Hello failed");
        if (reply) dbus_message_unref(reply);
        dbus_error_free(&err);
        dbus_connection_close(conn);
        dbus_connection_unref(conn);
        return nullptr;
    }
    dbus_message_unref(reply);
    DBusMessage *request = dbus_message_new_method_call(
        DBUS_SERVICE_DBUS, DBUS_PATH_DBUS, DBUS_INTERFACE_DBUS, "RequestName");
    dbus_uint32_t flags = DBUS_NAME_FLAG_DO_NOT_QUEUE;
    if (!request || !dbus_message_append_args(request, DBUS_TYPE_STRING, &NAME,
                                             DBUS_TYPE_UINT32, &flags, DBUS_TYPE_INVALID)) {
        if (request) dbus_message_unref(request);
        diagnostic(reason, capacity, "Cannot allocate D-Bus name request");
        dbus_connection_close(conn);
        dbus_connection_unref(conn);
        return nullptr;
    }
    reply = bus_call(conn, request, &err);
    dbus_uint32_t ownership = 0;
    const bool owned = reply && dbus_message_get_args(reply, &err, DBUS_TYPE_UINT32,
        &ownership, DBUS_TYPE_INVALID) && (ownership == DBUS_REQUEST_NAME_REPLY_PRIMARY_OWNER
        || ownership == DBUS_REQUEST_NAME_REPLY_ALREADY_OWNER);
    if (!owned) {
        diagnostic(reason, capacity, err.message ? err.message
            : "FileManager1 is already owned by another application");
        if (reply) dbus_message_unref(reply);
        dbus_error_free(&err);
        dbus_connection_close(conn);
        dbus_connection_unref(conn);
        return nullptr;
    }
    dbus_message_unref(reply);
    dbus_error_free(&err);
    return conn;
}

void files_fm_close(void *handle) {
    if (!handle) return;
    auto *conn = static_cast<DBusConnection *>(handle);
    dbus_connection_close(conn);
    dbus_connection_unref(conn);
}

// 1 = a validated typed call, 0 = idle or an immediate protocol reply,
// -1 = disconnect/allocation failure. The caller owns the returned handle.
int files_fm_next(void *handle, int wait_ms, FilesFMCall **out,
                  char *reason, std::size_t capacity) {
    *out = nullptr;
    auto *conn = static_cast<DBusConnection *>(handle);
    if (!dbus_connection_read_write(conn, wait_ms)) {
        diagnostic(reason, capacity, "D-Bus session disconnected");
        return -1;
    }
    DBusMessage *message = dbus_connection_pop_message(conn);
    if (!message) return 0;
    if (dbus_message_get_type(message) != DBUS_MESSAGE_TYPE_METHOD_CALL) {
        dbus_message_unref(message);
        return 0;
    }
    bool ok = true;
    const char *path = dbus_message_get_path(message);
    if (!path || std::strcmp(path, PATH)) {
        ok = error(conn, message, DBUS_ERROR_UNKNOWN_OBJECT, "Unknown FileManager1 object");
    } else if (dbus_message_is_method_call(message, DBUS_INTERFACE_INTROSPECTABLE, "Introspect")) {
        ok = dbus_message_has_signature(message, "")
            ? string_reply(conn, message, XML)
            : error(conn, message, DBUS_ERROR_INVALID_ARGS, "Introspect takes no arguments");
    } else if (dbus_message_is_method_call(message, DBUS_INTERFACE_PEER, "Ping")) {
        ok = dbus_message_has_signature(message, "") ? empty_reply(conn, message)
            : error(conn, message, DBUS_ERROR_INVALID_ARGS, "Ping takes no arguments");
    } else if (dbus_message_is_method_call(message, DBUS_INTERFACE_PEER, "GetMachineId")) {
        if (!dbus_message_has_signature(message, "")) {
            ok = error(conn, message, DBUS_ERROR_INVALID_ARGS, "GetMachineId takes no arguments");
        } else {
            char *machine = dbus_get_local_machine_id();
            ok = machine ? string_reply(conn, message, machine)
                : error(conn, message, DBUS_ERROR_FAILED, "Machine ID is unavailable");
            dbus_free(machine);
        }
    } else {
        int method = -1;
        if (dbus_message_is_method_call(message, NAME, "ShowFolders")) method = 0;
        if (dbus_message_is_method_call(message, NAME, "ShowItems")) method = 1;
        if (dbus_message_is_method_call(message, NAME, "ShowItemProperties")) method = 2;
        if (method < 0) {
            ok = error(conn, message, DBUS_ERROR_UNKNOWN_METHOD, "Unknown FileManager1 method");
        } else if (!dbus_message_has_signature(message, "ass")
                   || dbus_message_contains_unix_fds(message)) {
            ok = error(conn, message, DBUS_ERROR_INVALID_ARGS,
                       "Expected URI string array and startup ID, without file descriptors");
        } else {
            auto *call = new (std::nothrow) FilesFMCall{message, method, 0, {}, nullptr};
            if (!call) {
                dbus_message_unref(message);
                diagnostic(reason, capacity, "Cannot allocate FileManager1 call");
                return -1;
            }
            DBusMessageIter args, values;
            dbus_message_iter_init(message, &args);
            dbus_message_iter_recurse(&args, &values);
            std::size_t bytes = 0;
            bool bounded = true;
            while (dbus_message_iter_get_arg_type(&values) != DBUS_TYPE_INVALID) {
                const char *uri = nullptr;
                dbus_message_iter_get_basic(&values, &uri);
                const std::size_t size = std::strlen(uri);
                if (call->count == MAX_URIS || size > MAX_URI || size > MAX_TEXT - bytes) {
                    bounded = false;
                    break;
                }
                bytes += size;
                call->uris[call->count++] = uri;
                dbus_message_iter_next(&values);
            }
            dbus_message_iter_next(&args);
            dbus_message_iter_get_basic(&args, &call->startup);
            const std::size_t startup_size = std::strlen(call->startup);
            bounded = bounded && startup_size <= 4096 && startup_size <= MAX_TEXT - bytes;
            if (bounded) {
                *out = call;
                return 1;
            }
            delete call;
            ok = error(conn, message, DBUS_ERROR_LIMITS_EXCEEDED,
                       "FileManager1 request exceeds URI or text limits");
        }
    }
    dbus_message_unref(message);
    if (!ok) diagnostic(reason, capacity, "Cannot queue D-Bus reply");
    return ok ? 0 : -1;
}

int files_fm_method(const FilesFMCall *call) { return call->method; }
std::size_t files_fm_count(const FilesFMCall *call) { return call->count; }
const char *files_fm_uri(const FilesFMCall *call, std::size_t index) {
    return index < call->count ? call->uris[index] : nullptr;
}
const char *files_fm_startup(const FilesFMCall *call) { return call->startup; }
void files_fm_free_call(FilesFMCall *call) {
    if (!call) return;
    dbus_message_unref(call->message);
    delete call;
}
int files_fm_reply(void *handle, FilesFMCall *call, const char *name, const char *text) {
    auto *conn = static_cast<DBusConnection *>(handle);
    const bool ok = name ? error(conn, call->message, name, text ? text : "Request failed")
                         : empty_reply(conn, call->message);
    files_fm_free_call(call);
    return ok ? 1 : 0;
}
} // extern "C"
