# Repository map (line numbers at HEAD 8da326fe)

File sizes in lines:
  acpx_modal.rs
  acpx_modal/inspector.rs
  acpx_modal/detail.rs
    6957 total

## views/acpx_modal/inspector.rs: items (line: text)
  17:const ARTIFACT_MAX_BYTES: usize = 256 * 1024;
  18:const HORIZ_SCROLL_STEP: usize = 8;
  21:pub enum InspectorOutcome {
  32:enum TabKind {
  42:impl TabKind {
  43:    fn label(self) -> &'static str {
  57:struct TabEntry {
  67:pub struct InspectorState {
  78:impl InspectorState {
  80:    pub fn new(
  224:    pub fn handle_key(&mut self, key: &KeyEvent) -> InspectorOutcome {
  380:    pub fn scroll(&mut self, delta: isize) -> bool {
  399:    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, _now_ms: i64) {
  425:    pub fn shortcuts(&self) -> Vec<&'static str> {
  440:    fn max_scroll_y(&self) -> usize {
  451:    fn compute_visual_lines(
  490:    fn render_header(&self, buf: &mut Buffer, row: Rect, theme: &Theme) {
  554:    fn render_tabs(&self, buf: &mut Buffer, row: Rect, theme: &Theme) {
  581:    fn render_body(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme) {
  682:struct VisualLine {
  688:fn widest_source_line(text: &str) -> usize {
  695:fn centre_offset(block_w: usize, block_h: usize, width: usize, height: usize) -> (usize, usize) {
  702:fn outcome_color(outcome: Outcome, theme: &Theme) -> Color {
  711:fn read_artifact_tab(run_dir: &Path, artifact: &ArtifactRef) -> (String, bool) {
  727:fn build_meta_text(step: &Step, node: Option<&FlowNode>) -> String {
  816:fn fmt_utc_hms_ms(ms: i64) -> String {
  823:fn decimal_digits(mut n: usize) -> usize {
  835:fn wrap_line_by_width(line: &str, max_width: usize) -> Vec<String> {
  863:fn slice_from_col(line: &str, skip_cols: usize) -> String {
  885:#[cfg(test)]
  886:mod tests {
  892:    fn sample_shell_step() -> Step {
  928:    fn sample_acp_step() -> Step {
  953:    fn row_text(buf: &Buffer, y: u16) -> String {
  962:    fn shell_action_tab_set_and_empty_stderr() {
  995:    fn acp_step_tab_set() {
  1011:    fn failed_step_opens_on_error_tab() {
  1022:    fn digit_and_tab_cycle_selection() {
  1054:    fn per_tab_scroll_memory_and_clamping() {
  1100:    fn wrap_toggle_changes_visual_line_count() {
  1118:    fn wrapping_wide_cjk_chars_never_exceeds_width() {
  1135:    fn copy_returns_active_tab_full_text() {
  1144:    fn json_output_tab_reads_as_rows_and_copy_still_gives_raw_json() {
  1173:    fn string_output_tab_is_unchanged() {
  1187:    fn replay_only_on_step_with_conversation() {
  1203:    fn prev_next_and_back_keys() {
  1221:    fn missing_artifact_shows_error_text_in_tab() {
  1237:    fn render_100x20_has_header_and_tabs_and_0x0_does_not_panic() {
  1267:    fn render_text(text: &str, wrap: bool, w: u16, h: u16, keys: &[char]) -> Buffer {
  1286:    fn first_content_cell(buf: &Buffer) -> Option<(u16, u16)> {
  1295:    fn short_content_is_centred_as_one_block() {
  1318:    fn tall_content_starts_at_the_top_row() {
  1333:    fn wide_content_starts_at_the_left_column() {
  1339:    fn wrap_mode_centres_the_wrapped_lines() {
  1355:    fn tiny_areas_do_not_panic() {

## views/acpx_modal.rs: items (line: text)
  1:pub(crate) mod detail;
  2:pub(crate) mod inspector;
  3:pub(crate) mod ledger;
  4:pub(crate) mod modal_view;
  5:pub(crate) mod run;
  34:const MAX_TITLE_WIDTH: usize = 72;
  36:const PREVIEW_MIN_WIDTH: u16 = 120;
  38:const EXTRAS_LIMIT: usize = 80;
  41:pub enum Mode {
  49:pub struct ReplayRequest {
  57:pub enum AcpxOutcome {
  67:struct ModalNotice {
  72:impl ModalNotice {
  73:    fn warning(text: String) -> Self {
  78:pub struct AcpxModalState {
  106:impl AcpxModalState {
  107:    pub fn open() -> Self {
  111:    pub fn open_at(runs_base: Option<PathBuf>, sessions_base: Option<PathBuf>) -> Self {
  136:    pub fn open_run(&mut self, dir: &Path) -> bool {
  160:    pub fn mode(&self) -> Mode {
  164:    pub fn wants_poll(&self) -> bool {
  180:    pub fn poll(&mut self) -> bool {
  200:    pub fn wants_esc(&self) -> bool {
  204:    pub fn notice(&self) -> Option<&str> {
  209:    pub fn set_notice(&mut self, text: String) {
  213:    pub fn report_copy(&mut self, delivery: &crate::clipboard::CopyDelivery) {
  222:    pub fn handle_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
  264:    pub fn handle_mouse(&mut self, mouse: &MouseEvent) -> AcpxOutcome {
  312:    fn rescan_runs(&mut self) -> bool {
  326:    fn load_extras(&mut self, runs: &[RunSummary]) {
  355:    fn sync_preview(&mut self) {
  369:    fn rescan_sessions(&mut self) {
  378:    fn handle_ledger_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
  398:    fn watch_open_mirror_run(&mut self, dir: &Path, bundle_run_id: &str) {
  415:    pub fn release_open_mirror_run(&mut self) {
  425:    pub fn cancel_open_mirror_run(&mut self) -> bool {
  452:    fn poll_answerers(&mut self) -> bool {
  479:    fn answer_mirror_gate(&mut self, run_id: &str, choice_label: &str, choice_key: &str) -> AcpxOutcome {
  505:    fn handle_run_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
  634:    fn handle_step_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
  677:    fn open_step_at(&mut self, idx: usize) -> bool {
  700:    fn replay_step(&mut self, idx: usize) -> AcpxOutcome {
  726:    fn title_text(&self) -> String {
  750:    fn active_shortcut_labels(&self) -> Vec<&'static str> {
  776:fn row_extra(summary: &RunSummary) -> RowExtra {
  805:fn parse_gate_answer(input: &serde_json::Value) -> Option<GateAnswer> {
  822:fn read_run_answers(dir: &Path) -> Option<GateAnswer> {
  829:fn session_replay(s: SessionSummary) -> ReplayRequest {
  838:pub fn render_acpx_modal(
  948:fn render_notice_row(buf: &mut Buffer, content: Rect, notice: &ModalNotice, theme: &Theme) {
  969:const LEDGER_KEYS: &[(&str, &str)] = &[
  985:const RUN_KEYS: &[(&str, &str)] = &[
  1004:const STEP_KEYS: &[(&str, &str)] = &[
  1015:fn render_help_overlay(buf: &mut Buffer, area: Rect, mode: Mode, theme: &Theme) {
  1085:fn now_ms() -> i64 {
  1091:#[cfg(test)]
  1092:mod tests {
  1098:    fn key(code: KeyCode) -> KeyEvent {
  1102:    struct TestFixture {
  1111:    fn write_fixture(status: &str) -> TestFixture {
  1242:    fn buffer_text(buf: &Buffer) -> String {
  1255:    fn copy_notice_distinguishes_confirmed_clipboard_from_file_fallback() {
  1280:    fn open_at_lists_runs_and_sessions_and_navigates_modes() {
  1313:    fn wants_esc_reflects_mode_and_help_state() {
  1337:    fn replay_on_acp_step_returns_expected_request() {
  1362:    fn opening_session_in_ledger_returns_replay_with_none_range() {
  1383:    fn step_next_bracket_advances_and_esc_preserves_run_selection() {
  1413:    fn detail_focus_tab_backtab_and_without_detail_area() {
  1444:    fn detail_focus_range_copy_all_esc_unwind_and_tree_keys() {
  1515:    fn open_run_missing_dir_sets_notice_and_next_key_clears_it() {
  1534:    fn poll_detects_live_json_status_change_on_disk() {
  1567:    fn wants_poll_is_false_for_waiting_run() {
  1580:    fn question_mark_toggles_help_overlay_and_any_key_closes_it() {
  1604:    fn render_at_120x30_draws_title_and_tiny_sizes_do_not_panic() {
  1627:    fn row_extra_reads_gate_answer_and_handles_missing_or_malformed_gate() {
  1658:    fn run_outside_extras_limit_gets_answers_with_empty_rail() {
  1702:#[cfg(test)]
  1703:mod gate_answer_target_tests {
  1709:    fn key(code: KeyCode) -> KeyEvent {
  1715:    fn write_piw_gate_fixture() -> (tempfile::TempDir, PathBuf) {
  1771:    fn answer_target_is_parsed_and_routed_not_launched() {
  1808:    fn cancel_open_mirror_run_without_mirror_is_a_no_op() {

## views/acpx_modal/detail.rs: items (line: text)
  26:pub enum DetailTarget {
  33:struct CacheKey {
  45:struct CopyUnit {
  52:struct Rows {
  57:impl Rows {
  58:    fn new() -> Self {
  62:    fn push_unit(&mut self, rows: Vec<Line<'static>>, text: String) {
  72:    fn append_rows(&mut self, other: Rows) {
  82:    fn unit_at(&self, row: usize) -> Option<&CopyUnit> {
  89:    fn copy_range(&self, start_row: usize, end_row: usize) -> String {
  108:    fn copy_all(&self) -> String {
  117:impl std::ops::Deref for Rows {
  120:    fn deref(&self) -> &Self::Target {
  125:impl std::ops::DerefMut for Rows {
  126:    fn deref_mut(&mut self) -> &mut Self::Target {
  131:impl IntoIterator for Rows {
  135:    fn into_iter(self) -> Self::IntoIter {
  144:    fn into_iter(self) -> Self::IntoIter {
  149:fn line_plain_text(line: &Line<'_>) -> String {
  157:fn source_lines(text: &str) -> Vec<&str> {
  165:pub struct DetailPanel {
  180:impl Default for DetailPanel {
  181:    fn default() -> Self {
  186:impl DetailPanel {
  187:    pub fn new() -> Self {
  203:    pub fn set_answered(&mut self, answered: Option<&str>) {
  209:    pub fn is_focused(&self) -> bool {
  213:    pub fn is_visual(&self) -> bool {
  217:    pub fn set_focused(&mut self, focused: bool) {
  242:    pub fn page_step(&self) -> isize {
  252:    pub fn move_cursor(&mut self, delta: isize) -> bool {
  270:    pub fn toggle_visual(&mut self) -> bool {
  282:    pub fn handle_esc(&mut self) -> bool {
  294:    pub fn copy_selection(&self) -> Option<String> {
  299:    pub fn copy_all(&self) -> Option<String> {
  307:    fn selected_range(&self) -> Option<(usize, usize)> {
  322:    fn ensure_cursor_visible(&mut self, height: usize) {
  338:    pub fn scroll(&mut self, delta: isize) -> bool {
  360:    fn max_scroll(&self, height: usize) -> usize {
  368:    pub fn render(
  479:fn conversation_file_len(run: &Run, target: &DetailTarget) -> Option<u64> {
  503:fn build_lines(
  529:fn build_run_lines(
  719:fn step_one_line_summary(step: &Step) -> String {
  746:const EMPTY: &str = "(empty)";
  748:fn is_empty_container(val: &Value) -> bool {
  807:fn format_value_lines(val: &Value, width: usize, theme: &Theme) -> Rows {
  821:fn append_value_lines(
  986:fn first_non_empty_line(s: &str) -> String {
  994:fn build_step_lines(
  1013:fn build_step_lines_at(
  1314:fn failure_tail_for_step(run: &Run, step: &Step) -> Option<String> {
  1319:    const TAIL_ARTIFACT_MAX_BYTES: usize = 64 * 1024;
  1393:fn extract_step_note(step: &Step, verdict: Option<&StepVerdict>) -> Option<(String, bool)> {
  1394:    const NOTE_KEYS: &[&str] = &[
  1437:fn truncate_note_text(text: &str) -> (String, bool) {
  1438:    const MAX_LINES: usize = 3;
  1439:    const MAX_CHARS: usize = 240;
  1474:fn has_extra_result_fields(run: &Run, step: &Step, note_covered_entire_string: bool) -> bool {
  1494:fn is_digest_covered_key(key: &str, switch_on_key: Option<&str>) -> bool {
  1517:fn append_artifact_section(
  1525:    const MAX_BYTES: usize = 64 * 1024;
  1548:fn build_node_lines(
  1660:fn append_gate_section(
  1727:enum ConvItem {
  1738:fn append_conversation(
  1833:fn read_last_512k(path: &Path) -> std::io::Result<String> {
  1834:    const LIMIT: u64 = 512 * 1024;
  1853:fn parse_conversation_items(
  1997:fn format_trace_event(
  2020:fn push_section_rule(
  2045:fn title_row_right(
  2088:fn push_wrapped(
  2103:fn push_wrapped_capped(
  2134:fn dim_line(text: &str, theme: &Theme) -> Line<'static> {
  2141:fn truncate_line(line: Line<'static>, width: usize) -> Line<'static> {
  2161:fn run_status_color(status: RunStatus, theme: &Theme) -> Color {
  2171:fn node_state_color(state: NodeState, theme: &Theme) -> Color {
  2182:fn wrap(text: &str, width: usize) -> Vec<String> {
  2283:#[cfg(test)]
  2284:mod tests {
  2292:    fn make_test_run(dir: &Path) -> Run {
  2353:    fn buffer_lines(buf: &Buffer) -> Vec<String> {
  2367:    fn test_wrap_breaks_on_spaces_and_keeps_blank_lines() {
  2373:    fn test_wrap_hard_breaks_long_words_and_tabs() {
  2381:    fn test_wrap_wide_characters() {
  2388:    fn test_step_sections_order_prompt_result_events() {
  2440:    fn test_action_step_shows_command_exit_code_and_stdout() {
  2498:    fn test_conversation_thought_tool_call_and_message() {
  2567:    fn test_conversation_filters_seq_range_and_skips_auth_status_update() {
  2631:    fn test_missing_events_file_renders_dim_row() {
  2679:    fn test_run_target_lists_stages_and_waiting_gate_choices() {
  2749:    fn waiting_gate_run(dir: &Path) -> Run {
  2785:    fn panel_text(
  2799:    fn answered_gate_is_drawn_done_without_pickable_keys() {
  2820:    fn answered_gate_with_unknown_choice_ticks_nothing() {
  2831:    fn answer_arriving_redraws_a_cached_panel() {
  2850:    fn empty_list_and_object_values_read_as_empty_in_dim_style() {
  2908:    fn test_node_target_not_run() {
  2930:    fn test_scroll_clamps_and_returns_false_at_limits() {
  2975:    fn test_target_change_resets_scroll() {
  3022:    fn wrapped_failure_line_copies_exact_source_on_second_row() {
  3121:    fn focus_highlight_visual_range_and_target_change_reset_cursor() {
  3177:    fn test_failed_shell_orders_failure_tail_before_command_and_cwd_without_success() {
  3240:    fn test_repeated_attempt_shows_own_number_and_timing() {
  3299:    fn test_agent_with_brief_evidence_omits_prompt_wrappers_and_transcript_in_first_view() {
  3402:    fn test_agent_without_evidence_does_not_invent_it() {
  3438:    fn test_pending_gate_keeps_question_and_choices_conspicuous() {
  3458:    fn test_running_node_tail_follows_and_stops_on_scroll_up() {
  3570:    fn test_last_result_object_and_array_renders_clean_without_json_punctuation() {
  3635:    fn test_string_output_wraps_and_empty_or_null_hides_section() {
  3688:    fn test_array_output_of_40_items_renders_all_without_cap() {
  3723:    fn test_small_and_zero_rects_do_not_panic_and_stay_inside_area() {
