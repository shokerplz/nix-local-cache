import { useState, useEffect, useRef, useCallback } from 'react'
import axios from 'axios'
import { API_BASE_URL } from '../lib/config'

interface LogBatch {
    lines: string[]
    total_lines: number
    from_line: number
    returned_lines: number
}

interface BatchedLogsState {
    lines: string[]
    totalLines: number
    loadedRanges: Set<number>
    isLoading: boolean
    hasMore: boolean
}

export function useBatchedLogs(jobId: string, isJobRunning: boolean) {
    const [state, setState] = useState<BatchedLogsState>({
        lines: [],
        totalLines: 0,
        loadedRanges: new Set(),
        isLoading: false,
        hasMore: true,
    })
    const [liveLines, setLiveLines] = useState<string[]>([])
    const eventSourceRef = useRef<EventSource | null>(null)
    const BATCH_SIZE = 200

    // Load a batch of logs
    const loadBatch = useCallback(async (fromLine: number, tail: boolean = false) => {
        if (state.loadedRanges.has(fromLine) && !tail) {
            return
        }

        setState(prev => ({ ...prev, isLoading: true }))

        try {
            const params = new URLSearchParams({
                from_line: fromLine.toString(),
                limit: BATCH_SIZE.toString(),
                tail: tail.toString(),
            })

            const response = await axios.get<LogBatch>(
                `${API_BASE_URL}/jobs/${jobId}/logs/range?${params}`
            )

            const batch = response.data

            setState(prev => {
                const newLoadedRanges = new Set(prev.loadedRanges)
                newLoadedRanges.add(batch.from_line)

                // If this is the first load or tail request, replace lines
                if (tail || prev.lines.length === 0) {
                    return {
                        ...prev,
                        lines: batch.lines,
                        totalLines: batch.total_lines,
                        loadedRanges: newLoadedRanges,
                        isLoading: false,
                        hasMore: batch.from_line > 0,
                    }
                }

                // Prepend lines for scroll-up loading
                // Check if we're loading earlier content
                if (batch.from_line < prev.lines.length) {
                    return {
                        ...prev,
                        lines: [...batch.lines, ...prev.lines],
                        totalLines: batch.total_lines,
                        loadedRanges: newLoadedRanges,
                        isLoading: false,
                        hasMore: batch.from_line > 0,
                    }
                }

                return {
                    ...prev,
                    totalLines: batch.total_lines,
                    loadedRanges: newLoadedRanges,
                    isLoading: false,
                }
            })
        } catch (error) {
            console.error('Failed to load log batch:', error)
            setState(prev => ({ ...prev, isLoading: false }))
        }
    }, [jobId, state.loadedRanges])

    // Initial load - get the tail
    useEffect(() => {
        loadBatch(0, true)
    }, [jobId])

    // Setup SSE for running jobs
    useEffect(() => {
        if (!isJobRunning) {
            return
        }

        const eventSource = new EventSource(`${API_BASE_URL}/jobs/${jobId}/logs`)
        eventSourceRef.current = eventSource

        eventSource.onmessage = (event) => {
            setLiveLines(prev => [...prev, event.data])
        }

        eventSource.onerror = () => {
            eventSource.close()
        }

        return () => {
            eventSource.close()
        }
    }, [jobId, isJobRunning])

    // Load more when scrolling to top
    const loadMoreAbove = useCallback(() => {
        if (state.isLoading || !state.hasMore) {
            return
        }

        // Calculate the next batch to load
        const earliestLoadedLine = Math.min(...Array.from(state.loadedRanges))
        const nextFromLine = Math.max(0, earliestLoadedLine - BATCH_SIZE)

        loadBatch(nextFromLine, false)
    }, [state.isLoading, state.hasMore, state.loadedRanges, loadBatch])

    // Combine historical and live lines
    const allLines = [...state.lines, ...liveLines]

    return {
        lines: allLines,
        totalLines: Math.max(state.totalLines, allLines.length),
        isLoading: state.isLoading,
        hasMore: state.hasMore,
        loadMoreAbove,
    }
}
