// SPDX-License-Identifier: GPL-2.0-or-later
// Files' right-click action: "Share…" on one file on this computer. All it does is start
// wol-share with the file's path; the share sheet (the apps that can take it, Copy, Save a Copy,
// Open With) is the shell's own. Only a single local file is offered: a folder or a selection
// of several is not shared.
#include <nautilus-extension.h>

#define MENU_ID "WolosShare::share"
#define MENU_LABEL "Share…"
#define MENU_TIP "Send this file to an app, or copy it"
#define PATH_KEY "wolos-share-path"

typedef struct {
  GObject parent;
} WolosShare;
typedef struct {
  GObjectClass parent;
} WolosShareClass;

static GType share_type;

static void on_activate(NautilusMenuItem *item, gpointer unused) {
  (void)unused;
  const char *path = g_object_get_data(G_OBJECT(item), PATH_KEY);
  const char *argv[] = {"wol-share", path, NULL};
  g_autoptr(GError) err = NULL;
  if (!g_spawn_async(NULL, (char **)argv, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, NULL, &err))
    g_warning("wol-share: %s", err->message);
}

// The local path of the one selected file, or NULL for a folder, a remote item or several.
static char *local_file(GList *files) {
  if (files == NULL || files->next != NULL)
    return NULL;
  NautilusFileInfo *info = NAUTILUS_FILE_INFO(files->data);
  if (nautilus_file_info_is_directory(info))
    return NULL;
  g_autoptr(GFile) location = nautilus_file_info_get_location(info);
  return location != NULL ? g_file_get_path(location) : NULL;
}

static GList *get_file_items(NautilusMenuProvider *provider, GList *files) {
  (void)provider;
  char *path = local_file(files);
  if (path == NULL)
    return NULL;
  NautilusMenuItem *item = nautilus_menu_item_new(MENU_ID, MENU_LABEL, MENU_TIP, NULL);
  g_object_set_data_full(G_OBJECT(item), PATH_KEY, path, g_free);
  g_signal_connect(item, "activate", G_CALLBACK(on_activate), NULL);
  return g_list_append(NULL, item);
}

static void menu_iface(gpointer iface, gpointer unused) {
  (void)unused;
  ((NautilusMenuProviderInterface *)iface)->get_file_items = get_file_items;
}

static void instance_init(GTypeInstance *self, gpointer klass) {
  (void)self;
  (void)klass;
}
static void class_init(gpointer klass, gpointer data) {
  (void)klass;
  (void)data;
}

void nautilus_module_initialize(GTypeModule *module) {
  static const GTypeInfo info = {
      sizeof(WolosShareClass), NULL, NULL, class_init, NULL, NULL,
      sizeof(WolosShare),      0,    instance_init, NULL,
  };
  static const GInterfaceInfo menu = {menu_iface, NULL, NULL};
  share_type = g_type_module_register_type(module, G_TYPE_OBJECT, "WolosShare", &info, 0);
  g_type_module_add_interface(module, share_type, NAUTILUS_TYPE_MENU_PROVIDER, &menu);
}

void nautilus_module_shutdown(void) {}

void nautilus_module_list_types(const GType **types, int *count) {
  *types = &share_type;
  *count = 1;
}
