import { withCodSpeed } from "@codspeed/tinybench-plugin";
import { Bench } from "tinybench";

export const createBench = (options) => withCodSpeed(new Bench(options));

export const latencyMeanMs = (task) => task.result?.latency?.mean ?? task.result?.mean ?? 0;

export const latencyP99Ms = (task) => task.result?.latency?.p99 ?? task.result?.p99 ?? 0;

export const throughputHz = (task) => task.result?.throughput?.mean ?? task.result?.hz ?? 0;
