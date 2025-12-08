import { renderHook, act, waitFor } from '@testing-library/react'
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { useBatchedLogs } from './use-batched-logs'
import axios from 'axios'

// Mock axios
vi.mock('axios')
const mockedAxios = axios as unknown as { get: ReturnType<typeof vi.fn> }

// Mock EventSource
global.EventSource = vi.fn(() => ({
    onmessage: null,
    onerror: null,
    close: vi.fn(),
})) as any

describe('useBatchedLogs', () => {
    beforeEach(() => {
        vi.clearAllMocks()
    })

    it('should load initial tail logs and then prepend older logs on loadMoreAbove', async () => {
        // Setup initial tail response (lines 800-1000)
        mockedAxios.get.mockResolvedValueOnce({
            data: {
                lines: Array.from({ length: 200 }, (_, i) => `Line ${800 + i}`),
                total_lines: 1000,
                from_line: 800,
                returned_lines: 200
            }
        })

        const { result } = renderHook(() => useBatchedLogs('job-1', false))

        // Wait for initial load
        await waitFor(() => {
            expect(result.current.lines.length).toBe(200)
            expect(result.current.isLoading).toBe(false)
        })

        expect(result.current.lines[0]).toBe('Line 800')
        expect(result.current.lines[199]).toBe('Line 999')

        // Setup second response (lines 600-800) for loadMoreAbove
        mockedAxios.get.mockResolvedValueOnce({
            data: {
                lines: Array.from({ length: 200 }, (_, i) => `Line ${600 + i}`),
                total_lines: 1000,
                from_line: 600,
                returned_lines: 200
            }
        })

        // Trigger loadMoreAbove
        act(() => {
            result.current.loadMoreAbove()
        })

        // Wait for update
        await waitFor(() => {
            expect(result.current.isLoading).toBe(false)
        })
        
        // This expectation is expected to fail with the current bug
        // The bug prevents prepending, so length would remain 200
        expect(result.current.lines.length).toBe(400)
        expect(result.current.lines[0]).toBe('Line 600')
        expect(result.current.lines[200]).toBe('Line 800')
    })
})
