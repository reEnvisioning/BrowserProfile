// Startup
user_pref("browser.startup.page", 1);
user_pref("browser.startup.homepage", "about:home");

// Tabs — opening
user_pref("browser.link.open_newwindow", 3); // Open links in tabs.
user_pref("browser.tabs.loadInBackground", true); // Do not switch immediately.
user_pref("browser.link.open_newwindow.override.external", -1); // App links use normal placement.

// Tabs — interaction
user_pref("browser.ctrlTab.sortByRecentlyUsed", false);
user_pref("browser.tabs.hoverPreview.showThumbnails", true);
user_pref("browser.tabs.dragDrop.createGroup.enabled", true);

// Downloads and media
user_pref("browser.download.useDownloadDir", false);
user_pref("media.autoplay.default", 5);
user_pref("media.eme.enabled", false);

// Search
// Default engines are browser database state and cannot be declared in user.js.
user_pref("browser.urlbar.showSearchTerms.enabled", true);
user_pref("browser.search.separatePrivateDefault", true);
user_pref("browser.search.separatePrivateDefault.ui.enabled", true);
user_pref("browser.search.suggest.enabled", false);
user_pref("browser.urlbar.suggest.searches", false);
user_pref("browser.urlbar.showSearchSuggestionsFirst", true);
user_pref("browser.search.suggest.enabled.private", false);

// Address bar suggestions
user_pref("browser.urlbar.suggest.history", true);
user_pref("browser.urlbar.suggest.bookmark", true);
user_pref("browser.urlbar.suggest.openpage", true);
user_pref("browser.urlbar.suggest.topsites", true);
user_pref("browser.urlbar.suggest.recentsearches", true);
user_pref("browser.urlbar.suggest.engines", true);
user_pref("browser.urlbar.suggest.quickactions", true);

// Basic privacy
user_pref("browser.contentblocking.category", "strict");
user_pref("privacy.globalprivacycontrol.enabled", true);

// History and shutdown
user_pref("privacy.sanitize.sanitizeOnShutdown", true);
user_pref("privacy.clearOnShutdown_v2.browsingHistoryAndDownloads", false);
user_pref("browser.formfill.enable", false);
user_pref("browser.privatebrowsing.autostart", false);

// Passwords and autofill
user_pref("signon.rememberSignons", false);
user_pref("signon.autofillForms", false);
user_pref("extensions.formautofill.addresses.enabled", false);
user_pref("extensions.formautofill.creditCards.enabled", false);

// Basic HTTPS and DNS
user_pref("dom.security.https_only_mode", true);
user_pref("network.trr.mode", 5);

// Basic data collection
user_pref("datareporting.healthreport.uploadEnabled", false);
user_pref("app.shield.optoutstudies.enabled", false);

// Top tabs
user_pref("sidebar.verticalTabs", false);
user_pref("sidebar.visibility", "hide-on-close");
