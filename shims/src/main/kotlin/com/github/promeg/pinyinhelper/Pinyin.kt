package com.github.promeg.pinyinhelper

import net.sourceforge.pinyin4j.PinyinHelper
import net.sourceforge.pinyin4j.format.HanyuPinyinCaseType
import net.sourceforge.pinyin4j.format.HanyuPinyinOutputFormat
import net.sourceforge.pinyin4j.format.HanyuPinyinToneType

/**
 * TinyPinyin's JitPack artifact is Android-only, so this reimplements the one call upstream uses
 * (Pinyin.toPinyin(char)) on top of pinyin4j. Non-Chinese characters come back unchanged.
 */
object Pinyin {
    private val format = HanyuPinyinOutputFormat().apply {
        caseType = HanyuPinyinCaseType.UPPERCASE
        toneType = HanyuPinyinToneType.WITHOUT_TONE
    }

    @JvmStatic
    fun toPinyin(c: Char): String =
        runCatching { PinyinHelper.toHanyuPinyinStringArray(c, format)?.firstOrNull() }.getOrNull() ?: c.toString()
}
