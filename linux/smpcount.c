/* SPDX-License-Identifier: Apache-2.0 */
/* Two threads, each held to a CPU of its own, add to three counters
 * (issue 1408): one with an atomic add, which is amoadd.w, one with a
 * compare and swap, which is lr.w and sc.w, and one under a mutex, which
 * is musl's lock and futex. Each count must come out exact, and each
 * thread must have run on its own CPU, so the board's two harts and the
 * exclusive pairs between them are what kept every count. */
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>
#include <stdio.h>

#define N 20000

static unsigned added, swapped, locked;
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static unsigned seen[2];

static void *work(void *arg)
{
	int me = (int)(long)arg;
	cpu_set_t one;

	CPU_ZERO(&one);
	CPU_SET(me, &one);
	sched_setaffinity(0, sizeof one, &one);
	for (int i = 0; i < N; i++) {
		__atomic_fetch_add(&added, 1, __ATOMIC_RELAXED);
		unsigned v = __atomic_load_n(&swapped, __ATOMIC_RELAXED);
		while (!__atomic_compare_exchange_n(&swapped, &v, v + 1, 0,
						    __ATOMIC_RELAXED,
						    __ATOMIC_RELAXED))
			;
		pthread_mutex_lock(&lock);
		locked++;
		pthread_mutex_unlock(&lock);
		int c = sched_getcpu();
		if (c >= 0 && c < 32)
			seen[me] |= 1u << c;
	}
	return 0;
}

int main(void)
{
	pthread_t t[2];

	for (long i = 0; i < 2; i++)
		pthread_create(&t[i], 0, work, (void *)i);
	for (int i = 0; i < 2; i++)
		pthread_join(t[i], 0);
	printf("txhdl: smp count %u %u %u of %u, cpus %x %x\n", added,
	       swapped, locked, 2 * N, seen[0], seen[1]);
	if (added == 2 * N && swapped == 2 * N && locked == 2 * N &&
	    seen[0] == 1 && seen[1] == 2)
		printf("txhdl: smp count right on two harts\n");
	else
		printf("txhdl: smp count WRONG\n");
	return 0;
}
