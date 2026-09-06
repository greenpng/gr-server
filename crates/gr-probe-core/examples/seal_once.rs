
fn main() {
  let secret=b"lab-seal-secret-for-tests-only!!";
  let sid=std::env::args().nth(1).unwrap();
  let vt=std::env::args().nth(2).unwrap();
  let env=gr_probe_core::seal_probe_payload(secret,&sid,&vt,"B2_hardware",
    &serde_json::json!({"source":"main","fields":{"visitor_terminal_id":vt,"user_agent":"Mozilla/5.0","hardware_concurrency":4}})).unwrap();
  print!("{}", serde_json::to_string(&env).unwrap());
}
