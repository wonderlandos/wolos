// SPDX-License-Identifier: GPL-2.0-or-later
// Files' right-click action: "Check for Malware" on files on this computer. All it does is
// start wol-analyze with their paths, which asks first if the signatures are off.
#include <nautilus-extension.h>

#define MENU_ID "WolosAnalyze::analyze"
#define MENU_LABEL "Check for Malware"
#define MENU_TIP "Check the selected files with the full signature set"
#define PATHS_KEY "wolos-analyze-paths"

typedef struct {
  GObject parent;
} WolosAnalyze;
typedef struct {
  GObjectClass parent;
} WolosAnalyzeClass;

static GType analyze_type;

static void on_activate(NautilusMenuItem *item, gpointer unused) {
  (void)unused;
  GPtrArray *paths = g_object_get_data(G_OBJECT(item), PATHS_KEY);
  g_autoptr(GPtrArray) argv = g_ptr_array_new();
  g_ptr_array_add(argv, (gpointer) "wol-analyze");
  for (guint i = 0; i < paths->len; i++)
    g_ptr_array_add(argv, g_ptr_array_index(paths, i));
  g_ptr_array_add(argv, NULL);
  g_autoptr(GError) err = NULL;
  if (!g_spawn_async(NULL, (char **)argv->pdata, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, NULL, &err))
    g_warning("wol-analyze: %s", err->message);
}

// The paths of the selection, or NULL unless every item is a file with a local path.
static GPtrArray *local_files(GList *files) {
  GPtrArray *paths = g_ptr_array_new_with_free_func(g_free);
  for (GList *l = files; l != NULL; l = l->next) {
    NautilusFileInfo *info = NAUTILUS_FILE_INFO(l->data);
    g_autoptr(GFile) location = nautilus_file_info_get_location(info);
    char *path = location != NULL ? g_file_get_path(location) : NULL;
    if (path == NULL || nautilus_file_info_is_directory(info)) {
      g_free(path);
      g_ptr_array_unref(paths);
      return NULL;
    }
    g_ptr_array_add(paths, path);
  }
  if (paths->len == 0) {
    g_ptr_array_unref(paths);
    return NULL;
  }
  return paths;
}

static GList *get_file_items(NautilusMenuProvider *provider, GList *files) {
  (void)provider;
  GPtrArray *paths = local_files(files);
  if (paths == NULL)
    return NULL;
  NautilusMenuItem *item = nautilus_menu_item_new(MENU_ID, MENU_LABEL, MENU_TIP, NULL);
  g_object_set_data_full(G_OBJECT(item), PATHS_KEY, paths, (GDestroyNotify)g_ptr_array_unref);
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
      sizeof(WolosAnalyzeClass), NULL, NULL, class_init, NULL, NULL,
      sizeof(WolosAnalyze),      0,    instance_init, NULL,
  };
  static const GInterfaceInfo menu = {menu_iface, NULL, NULL};
  analyze_type = g_type_module_register_type(module, G_TYPE_OBJECT, "WolosAnalyze", &info, 0);
  g_type_module_add_interface(module, analyze_type, NAUTILUS_TYPE_MENU_PROVIDER, &menu);
}

void nautilus_module_shutdown(void) {}

void nautilus_module_list_types(const GType **types, int *count) {
  *types = &analyze_type;
  *count = 1;
}
