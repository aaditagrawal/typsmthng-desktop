/*
 * Optional no-op libayatana-appindicator3.so.1.
 *
 * Electrobun 1.15.1's libNativeWrapper.so lists this library as DT_NEEDED, so
 * a missing system package kills the process at load even though typsmthng
 * never creates a tray. Packaging prefers the real library; this stub is only
 * injected when that .so cannot be found.
 */
typedef void AppIndicator;

AppIndicator *app_indicator_new(const char *id, const char *icon_name, int category) {
	(void)id;
	(void)icon_name;
	(void)category;
	return 0;
}

void app_indicator_set_icon_full(AppIndicator *self, const char *icon_name, const char *icon_desc) {
	(void)self;
	(void)icon_name;
	(void)icon_desc;
}

void app_indicator_set_menu(AppIndicator *self, void *menu) {
	(void)self;
	(void)menu;
}

void app_indicator_set_status(AppIndicator *self, int status) {
	(void)self;
	(void)status;
}

void app_indicator_set_title(AppIndicator *self, const char *title) {
	(void)self;
	(void)title;
}
