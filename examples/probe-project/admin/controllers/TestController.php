<?php

namespace admin\controllers;

use App\Counter;
use Yii;

/**
 * Minimal stand-in for the owner's failing expressions (casts, ternaries, arrays, +=).
 * Source is analyzed as text only; nothing here is executed.
 */
class TestController
{
    public function actionProbe($ok, $data, $filter, $user, $documentId, $uid, $value)
    {
        // Owner line shape: cast must not destroy the call chain.
        $userId = (int)Yii::$app->request->post('userId');
        $flag = (bool)Yii::$app->request->post('flag');
        $label = (string)Yii::$app->request->post('label');

        $chosen = $ok ? \App\loadA() : \App\loadB();
        $elvis = $value ?: \App\fallback();
        $coalesce = $value ?? \App\fallback();

        $id = $data['id'];
        $deleted = [];
        $deleted['counter'] = Counter::deleteAll($filter);
        $deletedTotal = 0;
        $deletedTotal += Counter::deleteAll($filter);

        $payload = (object)[
            'status' => $ok ? 'valid' : 'invalid',
            'applicant' => (object)['id' => $user?->id ?? \App\fallback()],
            'document_id' => $documentId ?: ('sim-' . $uid),
        ];

        return [
            'userId' => $userId,
            'flag' => $flag,
            'label' => $label,
            'chosen' => $chosen,
            'elvis' => $elvis,
            'coalesce' => $coalesce,
            'id' => $id,
            'deleted' => $deleted,
            'deletedTotal' => $deletedTotal,
            'payload' => $payload,
        ];
    }
}
