export const benchmarkCopy = {
  en: {
    title: "Terminal benchmark", start: "Run benchmark", cancel: "Cancel", ready: "Ready",
    description: "Synthetic data only: 100 MiB, delayed ACK, and three rounds of 20 terminal sessions.",
    disabled: "Start a benchmark build with CAPSTAN_M0_REPORT_DIR to enable this tool.",
    failed: "Benchmark failed. Check the report and process samples; no acceptance is implied.",
    saved: "Report saved. Device acceptance remains a separate step.",
    bulk: "100 MiB", delayed_ack: "Delayed ACK", tab_lifecycle: "20 terminal sessions",
    round: (value: number) => `Round ${value}`, done: (count: number) => `${count} measured scenarios`,
    latency: (value: number | null) => `Input response p95: ${value === null ? "unmeasured" : `${value.toFixed(1)} ms`}`,
  },
  "zh-CN": {
    title: "终端基准", start: "运行基准", cancel: "取消", ready: "就绪",
    description: "仅使用合成数据：100 MiB、延迟 ACK，以及三轮 20 个终端会话。",
    disabled: "请用基准构建并设置 CAPSTAN_M0_REPORT_DIR 后启动此工具。",
    failed: "基准失败，请检查报告和进程采样；此结果不代表验收通过。",
    saved: "报告已保存，设备验收仍需单独完成。",
    bulk: "100 MiB", delayed_ack: "延迟 ACK", tab_lifecycle: "20 个终端会话",
    round: (value: number) => `第 ${value} 轮`, done: (count: number) => `已测量 ${count} 个场景`,
    latency: (value: number | null) => `输入响应 p95：${value === null ? "未测量" : `${value.toFixed(1)} ms`}`,
  },
} as const;
