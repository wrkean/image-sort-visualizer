//! Sorting algorithms recorded as a sequence of atomic events.
//!
//! Each algorithm runs to completion over a working array while emitting
//! `Event`s (comparisons, swaps, single-element writes). The recorded event
//! list can be replayed frame-by-frame to animate the exact operations.

use std::cmp::Ordering;

use clap::ValueEnum;

/// A single atomic operation performed by a sorting algorithm.
///
/// `a`/`b`/`idx` are positions in the linear cell array
/// (`idx = row * cols + col`).
#[derive(Clone, Copy, Debug)]
pub enum Event {
    /// Compare the keys of the cells currently at positions `a` and `b`.
    Cmp { a: usize, b: usize },
    /// Swap the cells at positions `a` and `b`.
    Swap { a: usize, b: usize },
    /// Write `value` (a cell id) into position `idx`.
    Set { idx: usize, value: usize },
}

/// The sorting algorithm to visualise.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Algo {
    #[value(alias = "bubblesort")]
    Bubble,
    #[value(alias = "insertionsort")]
    Insertion,
    #[value(alias = "selectionsort")]
    Selection,
    #[value(alias = "quicksort")]
    Quick,
    #[value(alias = "heapsort")]
    Heap,
    #[value(alias = "mergesort")]
    Merge,
}

impl Algo {
    pub const ALL: [Algo; 6] = [
        Algo::Bubble,
        Algo::Insertion,
        Algo::Selection,
        Algo::Quick,
        Algo::Heap,
        Algo::Merge,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Algo::Bubble => "Bubble sort",
            Algo::Insertion => "Insertion sort",
            Algo::Selection => "Selection sort",
            Algo::Quick => "Quicksort",
            Algo::Heap => "Heapsort",
            Algo::Merge => "Mergesort",
        }
    }

    pub fn next(self) -> Algo {
        let idx = Algo::ALL.iter().position(|&a| a == self).unwrap_or(0);
        Algo::ALL[(idx + 1) % Algo::ALL.len()]
    }

    pub fn prev(self) -> Algo {
        let idx = Algo::ALL.iter().position(|&a| a == self).unwrap_or(0);
        Algo::ALL[(idx + Algo::ALL.len() - 1) % Algo::ALL.len()]
    }
}

/// Run one of the algorithms over `arr`, emitting events, and record them.
///
/// `arr` is mutated in place to the sorted order (matching the events).
pub fn collect_events(
    algo: Algo,
    arr: &mut [usize],
    cmp: &dyn Fn(usize, usize) -> Ordering,
) -> Vec<Event> {
    let mut events = Vec::new();
    let mut emit = |e: Event| events.push(e);

    let n = arr.len();
    match algo {
        Algo::Bubble => bubble(arr, n, cmp, &mut emit),
        Algo::Insertion => insertion(arr, n, cmp, &mut emit),
        Algo::Selection => selection(arr, n, cmp, &mut emit),
        Algo::Quick => {
            if n > 1 {
                quick(arr, 0, n - 1, cmp, &mut emit);
            }
        }
        Algo::Heap => {
            if n > 1 {
                heap(arr, n, cmp, &mut emit);
            }
        }
        Algo::Merge => {
            if n > 1 {
                let mut aux = vec![0usize; n];
                merge(arr, &mut aux, 0, n, cmp, &mut emit);
            }
        }
    }

    events
}

fn bubble(arr: &mut [usize], n: usize, cmp: &dyn Fn(usize, usize) -> Ordering, emit: &mut impl FnMut(Event)) {
    for i in (1..n).rev() {
        for j in 0..i {
            emit(Event::Cmp { a: j, b: j + 1 });
            if cmp(arr[j], arr[j + 1]) == Ordering::Greater {
                arr.swap(j, j + 1);
                emit(Event::Swap { a: j, b: j + 1 });
            }
        }
    }
}

fn insertion(arr: &mut [usize], n: usize, cmp: &dyn Fn(usize, usize) -> Ordering, emit: &mut impl FnMut(Event)) {
    for i in 1..n {
        let mut j = i;
        while j > 0 {
            emit(Event::Cmp { a: j - 1, b: j });
            if cmp(arr[j - 1], arr[j]) == Ordering::Greater {
                arr.swap(j - 1, j);
                emit(Event::Swap { a: j - 1, b: j });
                j -= 1;
            } else {
                break;
            }
        }
    }
}

fn selection(arr: &mut [usize], n: usize, cmp: &dyn Fn(usize, usize) -> Ordering, emit: &mut impl FnMut(Event)) {
    for i in 0..n.saturating_sub(1) {
        let mut min = i;
        for j in (i + 1)..n {
            emit(Event::Cmp { a: min, b: j });
            if cmp(arr[j], arr[min]) == Ordering::Less {
                min = j;
            }
        }
        if min != i {
            arr.swap(i, min);
            emit(Event::Swap { a: i, b: min });
        }
    }
}

fn quick(arr: &mut [usize], lo: usize, hi: usize, cmp: &dyn Fn(usize, usize) -> Ordering, emit: &mut impl FnMut(Event)) {
    // Lomuto partition.
    let pivot = arr[hi];
    let mut i = lo;
    for j in lo..hi {
        emit(Event::Cmp { a: j, b: hi });
        if cmp(arr[j], pivot) == Ordering::Less {
            arr.swap(i, j);
            emit(Event::Swap { a: i, b: j });
            i += 1;
        }
    }
    if i != hi {
        arr.swap(i, hi);
        emit(Event::Swap { a: i, b: hi });
    }

    if i > lo {
        quick(arr, lo, i - 1, cmp, emit);
    }
    if i + 1 < hi {
        quick(arr, i + 1, hi, cmp, emit);
    }
}

fn heap(arr: &mut [usize], n: usize, cmp: &dyn Fn(usize, usize) -> Ordering, emit: &mut impl FnMut(Event)) {
    // Build max-heap.
    for start in (0..n / 2).rev() {
        sift_down(arr, start, n, cmp, emit);
    }
    // Extract repeatedly.
    for end in (1..n).rev() {
        arr.swap(0, end);
        emit(Event::Swap { a: 0, b: end });
        sift_down(arr, 0, end, cmp, emit);
    }
}

fn sift_down(arr: &mut [usize], mut root: usize, size: usize, cmp: &dyn Fn(usize, usize) -> Ordering, emit: &mut impl FnMut(Event)) {
    loop {
        let left = root * 2 + 1;
        if left >= size {
            break;
        }
        let right = left + 1;

        let mut largest = root;
        emit(Event::Cmp { a: left, b: root });
        if cmp(arr[left], arr[largest]) == Ordering::Greater {
            largest = left;
        }
        if right < size {
            emit(Event::Cmp { a: right, b: largest });
            if cmp(arr[right], arr[largest]) == Ordering::Greater {
                largest = right;
            }
        }

        if largest == root {
            break;
        }
        arr.swap(root, largest);
        emit(Event::Swap { a: root, b: largest });
        root = largest;
    }
}

fn merge(arr: &mut [usize], aux: &mut [usize], lo: usize, hi: usize, cmp: &dyn Fn(usize, usize) -> Ordering, emit: &mut impl FnMut(Event)) {
    if hi - lo < 2 {
        return;
    }
    let mid = lo + (hi - lo) / 2;
    merge(arr, aux, lo, mid, cmp, emit);
    merge(arr, aux, mid, hi, cmp, emit);

    // Two-way merge into the auxiliary buffer.
    let (mut i, mut j, mut k) = (lo, mid, lo);
    while i < mid && j < hi {
        emit(Event::Cmp { a: i, b: j });
        if cmp(arr[i], arr[j]) == Ordering::Less {
            aux[k] = arr[i];
            emit(Event::Set { idx: k, value: arr[i] });
            i += 1;
        } else {
            aux[k] = arr[j];
            emit(Event::Set { idx: k, value: arr[j] });
            j += 1;
        }
        k += 1;
    }
    while i < mid {
        aux[k] = arr[i];
        emit(Event::Set { idx: k, value: arr[i] });
        i += 1;
        k += 1;
    }
    while j < hi {
        aux[k] = arr[j];
        emit(Event::Set { idx: k, value: arr[j] });
        j += 1;
        k += 1;
    }

    // Copy the merged run back into `arr`.
    for (offset, &value) in aux[lo..hi].iter().enumerate() {
        let idx = lo + offset;
        arr[idx] = value;
        emit(Event::Set { idx, value });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify every algorithm produces (and replays to) a sorted array.
    #[test]
    fn all_algos_sort_and_replay() {
        for &algo in &Algo::ALL {
            for n in [0usize, 1, 2, 3, 5, 17, 64, 200] {
                let mut arr: Vec<usize> = (0..n).rev().collect();
                let cmp = |a: usize, b: usize| a.cmp(&b);
                let events = collect_events(algo, &mut arr, &cmp);

                assert!(
                    arr.windows(2).all(|w| w[0] <= w[1]),
                    "{algo:?} n={n}: array not sorted"
                );

                // Replay the events over a fresh copy "screen-side" array.
                let mut vis: Vec<usize> = (0..n).rev().collect();
                for e in &events {
                    match *e {
                        Event::Cmp { .. } => {}
                        Event::Swap { a, b } => vis.swap(a, b),
                        Event::Set { idx, value } => vis[idx] = value,
                    }
                }
                assert_eq!(vis, arr, "{algo:?} n={n}: replay diverged");
            }
        }
    }

    #[test]
    fn collects_events_on_reverse_input() {
        let mut arr: Vec<usize> = (0..20).rev().collect();
        let duration = std::time::Instant::now();
        let events = collect_events(Algo::Quick, &mut arr, &|a, b| a.cmp(&b));
        assert!(!events.is_empty());
        assert!(duration.elapsed().as_secs() < 5);
    }
}