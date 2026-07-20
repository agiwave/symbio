/**
 * 日志工具
 * 
 * 提供统一的日志级别控制，生产环境自动过滤调试日志
 */

// 日志级别
export enum LogLevel {
  ERROR = 0,
  WARN = 1,
  INFO = 2,
  DEBUG = 3
}

// 从环境变量读取日志级别，生产环境默认为 ERROR，开发环境默认为 DEBUG
function getLogLevel(): LogLevel {
  const envLevel = import.meta.env.VITE_LOG_LEVEL
  
  if (envLevel) {
    const levelMap: Record<string, LogLevel> = {
      'error': LogLevel.ERROR,
      'warn': LogLevel.WARN,
      'info': LogLevel.INFO,
      'debug': LogLevel.DEBUG
    }
    return levelMap[envLevel.toLowerCase()] ?? LogLevel.DEBUG
  }
  
  // 默认值
  return import.meta.env.PROD ? LogLevel.ERROR : LogLevel.DEBUG
}

const CURRENT_LEVEL = getLogLevel()

/**
 * 日志工具类
 */
export const logger = {
  /**
   * 调试日志（仅开发环境）
   */
  debug(module: string, message: string, ...args: unknown[]) {
    if (CURRENT_LEVEL >= LogLevel.DEBUG) {
      console.debug(`[${module}] ${message}`, ...args)
    }
  },

  /**
   * 信息日志
   */
  info(module: string, message: string, ...args: unknown[]) {
    if (CURRENT_LEVEL >= LogLevel.INFO) {
      console.info(`[${module}] ${message}`, ...args)
    }
  },

  /**
   * 警告日志
   */
  warn(module: string, message: string, ...args: unknown[]) {
    if (CURRENT_LEVEL >= LogLevel.WARN) {
      console.warn(`[${module}] ${message}`, ...args)
    }
  },

  /**
   * 错误日志（始终显示）
   */
  error(module: string, message: string, ...args: unknown[]) {
    if (CURRENT_LEVEL >= LogLevel.ERROR) {
      console.error(`[${module}] ${message}`, ...args)
    }
  }
}
