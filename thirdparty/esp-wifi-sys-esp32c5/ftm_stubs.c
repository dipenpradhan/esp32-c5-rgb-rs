// Stub symbols for FTM (Fine Timing Measurement) - not needed for basic WiFi STA
// These satisfy linker references from new libnet80211.a blob
// that expects newer libphy.a with FTM support.

// INIT variants
void est_PHY_INIT_FTM_COMP_20_20D_MHZ(void) {}
void est_PHY_INIT_FTM_COMP_20_20D_MHZ_DIS(void) {}
void est_PHY_INIT_FTM_COMP_20_20_MHZ_5G(void) {}
void est_PHY_INIT_FTM_COMP_20_20_MHZ_5G_DIS(void) {}
void est_PHY_INIT_FTM_COMP_20_20U_MHZ(void) {}
void est_PHY_INIT_FTM_COMP_20_20U_MHZ_DIS(void) {}
void est_PHY_INIT_FTM_COMP_20_40D_MHZ(void) {}
void est_PHY_INIT_FTM_COMP_20_40D_MHZ_DIS(void) {}
void est_PHY_INIT_FTM_COMP_20_40_MHZ_5G(void) {}
void est_PHY_INIT_FTM_COMP_20_40_MHZ_5G_DIS(void) {}
void est_PHY_INIT_FTM_COMP_20_40U_MHZ(void) {}
void est_PHY_INIT_FTM_COMP_20_40U_MHZ_DIS(void) {}
void est_PHY_INIT_FTM_COMP_40_40D_MHZ(void) {}
void est_PHY_INIT_FTM_COMP_40_40D_MHZ_DIS(void) {}
void est_PHY_INIT_FTM_COMP_40_40_MHZ_5G(void) {}
void est_PHY_INIT_FTM_COMP_40_40_MHZ_5G_DIS(void) {}
void est_PHY_INIT_FTM_COMP_40_40U_MHZ(void) {}
void est_PHY_INIT_FTM_COMP_40_40U_MHZ_DIS(void) {}
void est_PHY_RESP_FTM_COMP_40_40_MHZ_5G(void) {}
void est_PHY_RESP_FTM_COMP_40_40_MHZ_5G_DIS(void) {}

// RESP variants (new in updated blob)
void est_PHY_RESP_FTM_COMP_20_20D_MHZ(void) {}
void est_PHY_RESP_FTM_COMP_20_20D_MHZ_DIS(void) {}
void est_PHY_RESP_FTM_COMP_20_20_MHZ_5G(void) {}
void est_PHY_RESP_FTM_COMP_20_20_MHZ_5G_DIS(void) {}
void est_PHY_RESP_FTM_COMP_20_20U_MHZ(void) {}
void est_PHY_RESP_FTM_COMP_20_20U_MHZ_DIS(void) {}
void est_PHY_RESP_FTM_COMP_20_40D_MHZ(void) {}
void est_PHY_RESP_FTM_COMP_20_40D_MHZ_DIS(void) {}
void est_PHY_RESP_FTM_COMP_20_40_MHZ_5G(void) {}
void est_PHY_RESP_FTM_COMP_20_40_MHZ_5G_DIS(void) {}
void est_PHY_RESP_FTM_COMP_20_40U_MHZ(void) {}
void est_PHY_RESP_FTM_COMP_20_40U_MHZ_DIS(void) {}
void est_PHY_RESP_FTM_COMP_40_40D_MHZ(void) {}
void est_PHY_RESP_FTM_COMP_40_40D_MHZ_DIS(void) {}
void est_PHY_RESP_FTM_COMP_40_40U_MHZ(void) {}
void est_PHY_RESP_FTM_COMP_40_40U_MHZ_DIS(void) {}

// WPA supplicant stubs
int esp_wifi_skip_supp_pmkcaching(void) { return 0; }
void* esp_wifi_sta_get_rsnxe(void) { return 0; }
