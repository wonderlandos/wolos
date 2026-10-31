/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * scx_wolos: WolOS QoS classes on sched_ext (port of the FreeBSD wolos_qos policy).
 * The class is read from what userspace set with sched_setattr(2) (<linux/wolos.h>), see
 * scx_wolos_class.h: SCHED_IDLE -> background; uclamp_min >= 768 -> interactive; >= 384 ->
 * foreground (the frontmost app); uclamp_max <= 512 -> utility; else default. EEVDF reads the
 * same nice/uclamp values, so the classes keep meaning when this scheduler is not loaded.
 * Order: interactive (1 ms slices) > foreground (2 ms) > default/utility (4 ms, per-CPU vtime
 * queues, idle CPUs steal) > background (only when idle, or after bg_starve_ns). A waking
 * interactive or foreground task preempts a CPU running lower-class work.
 */
#include <scx/common.bpf.h>
#include "scx_wolos_class.h"

char _license[] SEC("license") = "GPL";

#define MAX_CPUS	512

const volatile u64 bg_starve_ns = 100000000ULL;		/* background runs at least every 100 ms */
/* schedutil target per class (0..1024); HWP/EPP drivers decide on their own. */
static const u32 class_perf[C_NR] = { 1024, 1024, 768, 512, 256 };

static u64 vtime_now;
static u64 bg_last_ns;
UEI_DEFINE(uei);

struct {
	__uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
	__uint(key_size, sizeof(u32));
	__uint(value_size, sizeof(u64));
	__uint(max_entries, C_NR);
} stats SEC(".maps");	/* enqueues per class, read by `scx_wolos --stats` */

/* Class of the task each CPU runs now (C_NR = idle), for preemption. */
struct {
	__uint(type, BPF_MAP_TYPE_ARRAY);
	__uint(key_size, sizeof(u32));
	__uint(value_size, sizeof(u32));
	__uint(max_entries, MAX_CPUS);
} cur_class SEC(".maps");

static __always_inline int task_class(const struct task_struct *p)
{
	return wolos_class(p->policy, p->uclamp_req[0].value, p->uclamp_req[1].value);
}

static void stat_inc(u32 c)
{
	u64 *n = bpf_map_lookup_elem(&stats, &c);

	if (n)
		(*n)++;
}

static u32 running_class(s32 cpu)
{
	u32 k = cpu, *c = bpf_map_lookup_elem(&cur_class, &k);

	return c ? *c : C_NR;
}

static void set_running_class(s32 cpu, u32 v)
{
	u32 k = cpu, *c = bpf_map_lookup_elem(&cur_class, &k);

	if (c)
		*c = v;
}

/* Preempt the task's CPU if it runs lower-class work, else the first CPU that does. */
static void preempt_lower(const struct task_struct *p, int c)
{
	s32 cpu = scx_bpf_task_cpu(p), i;
	u32 n = scx_bpf_nr_cpu_ids();

	if (wolos_class_preempts(c, running_class(cpu))) {
		scx_bpf_kick_cpu(cpu, SCX_KICK_PREEMPT);
		return;
	}
	bpf_for(i, 0, n) {
		if (i >= MAX_CPUS)
			break;
		if (!bpf_cpumask_test_cpu(i, p->cpus_ptr))
			continue;
		if (wolos_class_preempts(c, running_class(i))) {
			scx_bpf_kick_cpu(i, SCX_KICK_PREEMPT);
			return;
		}
	}
}

s32 BPF_STRUCT_OPS(wolos_select_cpu, struct task_struct *p, s32 prev_cpu, u64 wake_flags)
{
	bool idle = false;
	s32 cpu = scx_bpf_select_cpu_dfl(p, prev_cpu, wake_flags, &idle);

	if (idle) {
		int c = task_class(p);

		stat_inc(c);
		scx_bpf_dsq_insert(p, SCX_DSQ_LOCAL, wolos_class_slice(c), 0);
	}
	return cpu;
}

void BPF_STRUCT_OPS(wolos_enqueue, struct task_struct *p, u64 enq_flags)
{
	int c = task_class(p);
	int dsq = wolos_class_dsq(c);
	u64 vtime = p->scx.dsq_vtime;
	u64 slice = wolos_class_slice(c);

	stat_inc(c);
	/* An idle task keeps at most one slice of credit. */
	if (time_before(vtime, vtime_now - slice))
		vtime = vtime_now - slice;
	if (dsq == DSQ_PCPU) {
		/* Normal work stays on the CPU select_cpu picked: cache-hot, no shared lock. */
		s32 cpu = scx_bpf_task_cpu(p);

		scx_bpf_dsq_insert_vtime(p, DSQ_PCPU_BASE + cpu, slice, vtime, enq_flags);
		scx_bpf_kick_cpu(cpu, SCX_KICK_IDLE);
		return;
	}
	scx_bpf_dsq_insert_vtime(p, dsq, slice, vtime, enq_flags);
	if (c <= C_FOREGROUND)
		preempt_lower(p, c);
}

void BPF_STRUCT_OPS(wolos_dispatch, s32 cpu, struct task_struct *prev)
{
	u64 now = bpf_ktime_get_ns();
	u32 n = scx_bpf_nr_cpu_ids();
	s32 i;

	if (now - bg_last_ns > bg_starve_ns && scx_bpf_dsq_move_to_local(DSQ_BG, 0)) {
		bg_last_ns = now;
		return;
	}
	if (scx_bpf_dsq_move_to_local(DSQ_INTERACTIVE, 0) ||
	    scx_bpf_dsq_move_to_local(DSQ_FG, 0) ||
	    scx_bpf_dsq_move_to_local(DSQ_PCPU_BASE + cpu, 0))
		return;
	/* Own queue empty: steal normal work from the next busy CPU. */
	bpf_for(i, 1, n) {
		s32 v = (cpu + i) % n;

		if (scx_bpf_dsq_nr_queued(DSQ_PCPU_BASE + v) &&
		    scx_bpf_dsq_move_to_local(DSQ_PCPU_BASE + v, 0))
			return;
	}
	if (scx_bpf_dsq_move_to_local(DSQ_BG, 0))
		bg_last_ns = now;
}

void BPF_STRUCT_OPS(wolos_running, struct task_struct *p)
{
	int c = task_class(p);
	s32 cpu = scx_bpf_task_cpu(p);

	if (time_before(vtime_now, p->scx.dsq_vtime))
		vtime_now = p->scx.dsq_vtime;
	set_running_class(cpu, c);
	if (c >= 0 && c < C_NR)
		scx_bpf_cpuperf_set(cpu, class_perf[c]);
}

void BPF_STRUCT_OPS(wolos_stopping, struct task_struct *p, bool runnable)
{
	int c = task_class(p);
	u64 full = wolos_class_slice(c);
	u64 used = full > p->scx.slice ? full - p->scx.slice : 0;
	u64 delta = scale_by_task_weight_inverse(p, used);

	if (c == C_UTILITY)
		delta *= 2;
	scx_bpf_task_set_dsq_vtime(p, p->scx.dsq_vtime + delta);
	set_running_class(scx_bpf_task_cpu(p), C_NR);
}

void BPF_STRUCT_OPS(wolos_enable, struct task_struct *p)
{
	scx_bpf_task_set_dsq_vtime(p, vtime_now);
}

s32 BPF_STRUCT_OPS_SLEEPABLE(wolos_init)
{
	u32 n = scx_bpf_nr_cpu_ids();
	s32 i;
	int ret;

	ret = scx_bpf_create_dsq(DSQ_INTERACTIVE, -1);
	if (!ret)
		ret = scx_bpf_create_dsq(DSQ_FG, -1);
	if (!ret)
		ret = scx_bpf_create_dsq(DSQ_BG, -1);
	bpf_for(i, 0, n) {
		if (ret)
			break;
		ret = scx_bpf_create_dsq(DSQ_PCPU_BASE + i, -1);
	}
	if (ret)
		scx_bpf_error("create DSQ failed (%d)", ret);
	return ret;
}

void BPF_STRUCT_OPS(wolos_exit, struct scx_exit_info *ei)
{
	UEI_RECORD(uei, ei);
}

SCX_OPS_DEFINE(wolos_ops,
	       .select_cpu	= (void *)wolos_select_cpu,
	       .enqueue		= (void *)wolos_enqueue,
	       .dispatch	= (void *)wolos_dispatch,
	       .running		= (void *)wolos_running,
	       .stopping	= (void *)wolos_stopping,
	       .enable		= (void *)wolos_enable,
	       .init		= (void *)wolos_init,
	       .exit		= (void *)wolos_exit,
	       .timeout_ms	= 5000,
	       .name		= "wolos");
